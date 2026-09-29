#!/usr/bin/env python3
"""出包 → 发到 Gitee 发行版 → 更新 latest.json（在线更新的清单）。

用法：

    python3 scripts/release.py

就这一条。跑起来会列出当前版本和已有产物，然后问你要做什么——
发布 / 只构建 / 复用产物直接上传 / 改版本号。不用记命令行参数。

macOS 和 Windows 两个平台的包都在这台 Mac 上打：Windows 的是用 MinGW 交叉编译出来的
（环境怎么装见 docs/RELEASE.md「打包环境」）。发布时默认两个平台都传，进同一个发行版、写进同一份
latest.json；也可以选只发其中一个。

Gitee 令牌不用每次贴：环境变量 GITEE_TOKEN → ~/.switchboard-keys/gitee.token →
都没有就当场问你要（输入不回显），验证通过后可以存起来，下次直接用。
令牌存在仓库外、权限 0600，不会进代码库。

── 为什么是 Python 不是 shell ────────────────────────────────────────
上一版是 bash，里面套 `$(python3 -c "...")` 拼 JSON，再套一层双引号传给 curl -d。
嵌套引号把字典的花括号吃掉了，每个字段被当成独立命令执行。JSON + 多层引号这种组合
在 shell 里是自找麻烦，整个搬到 Python 里，只用标准库，没有引号可踩。

── 为什么是 universal 不是 Docker ────────────────────────────────────
macOS 的 .app/.dmg 只能在 macOS 上打：要 Apple SDK，要 hdiutil/codesign/sips，
Apple 也不允许在非苹果硬件的容器里跑 macOS。所以"多架构"在这里的正解是
universal-apple-darwin：一个包同时含 arm64 和 x86_64。

── 只发行版、不传源码 ────────────────────────────────────────────────
只做两件事：创建发行版并上传产物、写一个 latest.json 清单。源码不会被推上去。
"""
import base64
import datetime
import getpass
import json
import mimetypes
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path

OWNER, REPO, BRANCH = "etn", "switchboard_store", "master"
API = "https://gitee.com/api/v5"
ROOT = Path(__file__).resolve().parent.parent
KEY_PATH = Path.home() / ".switchboard-keys" / "updater.key"
TOKEN_FILE = Path.home() / ".switchboard-keys" / "gitee.token"
BUNDLE = ROOT / "src-tauri/target/universal-apple-darwin/release/bundle"
WIN_TARGET = "x86_64-pc-windows-gnu"
# 本机交叉编译出来的 Windows 安装包
WIN_BUNDLE = ROOT / f"src-tauri/target/{WIN_TARGET}/release/bundle/nsis"
# 在 Windows 电脑上手动打好的安装包拷到这里，发布时一样会被带上（见 docs/RELEASE.md「Windows 版」）
WIN_DROP = ROOT / "src-tauri/target/windows-nsis"


def die(msg):
    print(f"✗ {msg}")
    sys.exit(1)


def load_token():
    """找令牌：环境变量 → 本地文件 → 问用户。返回 (令牌, 是不是刚输的)。"""
    t = os.environ.get("GITEE_TOKEN", "").strip()
    if t:
        return t, False
    if TOKEN_FILE.exists():
        t = TOKEN_FILE.read_text().strip()
        if t:
            return t, False

    print("没找到 Gitee 令牌。怎么拿：")
    print("  1. 打开 https://gitee.com/personal_access_tokens")
    print("     （或 Gitee 右上角头像 → 设置 → 左侧「私人令牌」）")
    print("  2. 生成新令牌，权限至少勾上 projects")
    print("  3. 输密码确认后复制——它只显示这一次")
    # getpass：不回显、不进 shell 历史
    t = getpass.getpass("粘贴令牌（输入不会显示）：").strip()
    if not t:
        die("没输入令牌")
    return t, True


def save_token(token):
    TOKEN_FILE.parent.mkdir(parents=True, exist_ok=True)
    TOKEN_FILE.write_text(token)
    TOKEN_FILE.chmod(0o600)  # 只有自己读得到
    print(f"  已记住，下次不用再输：{TOKEN_FILE}")


def check_token(token):
    """开跑前先验一次。不然构建两分钟后才发现令牌不对，白等。"""
    me = api("GET", "/user", token=token)
    if "_status" in me or not me.get("login"):
        die(
            f"令牌无效或权限不足（{me.get('_status', '?')}）。\n"
            f"  确认生成时勾了 projects 权限。\n"
            f"  如果之前存过一个错的：rm {TOKEN_FILE} 然后重跑，会重新问你要。"
        )
    return me["login"]


def api(method, path, *, token, body=None, params=""):
    url = f"{API}{path}{params}"
    data = None
    headers = {}
    if body is not None:
        body = {**body, "access_token": token}
        data = json.dumps(body).encode()
        headers["Content-Type"] = "application/json"
    elif "access_token" not in url:
        url += ("&" if "?" in url else "?") + f"access_token={token}"
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            raw = r.read().decode()
    except urllib.error.HTTPError as e:
        return {"_status": e.code, "_body": e.read().decode()[:500]}
    except urllib.error.URLError as e:
        return {"_status": 0, "_body": f"网络不可达：{e.reason}"}

    if not raw.strip():
        return {}
    try:
        parsed = json.loads(raw)
    except json.JSONDecodeError:
        return {"_status": 0, "_body": f"返回的不是 JSON：{raw[:200]}"}

    # Gitee 查一个**不存在的 tag** 时，返回的是 HTTP 200 + 响应体 `null`，
    # 既不是 404 也不是 {}。直接 .get() 会 AttributeError——
    # 而这恰好是"第一次发版"必走的路径（那个 tag 当然还不存在）。
    # 所以这里统一归一成字典，调用方永远能安全 .get()。
    return parsed if isinstance(parsed, dict) else {"_raw": parsed}


def upload_attachment(release_id, path: Path, token):
    """multipart 上传。Gitee 的附件 URL 事先猜不出来，必须上传后从响应里回读。"""
    boundary = uuid.uuid4().hex
    ctype = mimetypes.guess_type(path.name)[0] or "application/octet-stream"
    body = b"".join([
        f"--{boundary}\r\n".encode(),
        f'Content-Disposition: form-data; name="file"; filename="{path.name}"\r\n'.encode(),
        f"Content-Type: {ctype}\r\n\r\n".encode(),
        path.read_bytes(),
        f"\r\n--{boundary}--\r\n".encode(),
    ])
    req = urllib.request.Request(
        f"{API}/repos/{OWNER}/{REPO}/releases/{release_id}/attach_files?access_token={token}",
        data=body,
        headers={"Content-Type": f"multipart/form-data; boundary={boundary}"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req) as r:
            return json.loads(r.read().decode())
    except urllib.error.HTTPError as e:
        die(f"上传 {path.name} 失败 HTTP {e.code}：{e.read().decode()[:300]}")


def read_version():
    return json.loads((ROOT / "src-tauri/tauri.conf.json").read_text())["version"]


def write_version(v):
    p = ROOT / "src-tauri/tauri.conf.json"
    d = json.loads(p.read_text())
    d["version"] = v
    p.write_text(json.dumps(d, indent=2, ensure_ascii=False) + "\n")


def find_artifacts():
    """返回 (dmg, tarball, sig)，缺任何一个就返回 None。"""
    dmgs = sorted((BUNDLE / "dmg").glob("*.dmg")) if (BUNDLE / "dmg").is_dir() else []
    tars = sorted((BUNDLE / "macos").glob("*.app.tar.gz")) if (BUNDLE / "macos").is_dir() else []
    if not dmgs or not tars:
        return None
    sig = tars[0].with_suffix(tars[0].suffix + ".sig")
    return (dmgs[0], tars[0], sig) if sig.exists() else None


def find_windows(version):
    """返回这个版本的 (setup.exe, .sig)，没有就 None。

    版本号从文件名里认：tauri 按 tauri.conf.json 的版本给安装包起名，Mac 包也是，
    所以只要文件名里的版本跟当前一致，两个平台就是同一个版本。版本不对的一律不算。
    """
    exes = [p for d in (WIN_BUNDLE, WIN_DROP) if d.is_dir() for p in d.rglob(f"*_{version}_x64-setup.exe")]
    exes = [p for p in exes if p.with_name(p.name + ".sig").exists()]
    if not exes:
        return None
    exe = max(exes, key=lambda p: p.stat().st_mtime)  # 两处都有就用新的那个
    return exe, exe.with_name(exe.name + ".sig")


def windows_tools_missing():
    """交叉编译 Windows 包要的三样，缺哪样返回哪样的安装命令"""
    missing = []
    if not shutil.which("x86_64-w64-mingw32-gcc"):
        missing.append("brew install mingw-w64")
    if not shutil.which("makensis"):
        missing.append("brew install makensis")
    installed = subprocess.run(["rustup", "target", "list", "--installed"], capture_output=True, text=True).stdout
    if WIN_TARGET not in installed:
        missing.append(f"rustup target add {WIN_TARGET}")
    return missing


def build_windows():
    """在这台 Mac 上交叉编译 Windows 安装包（NSIS），成功返回 True。

    为什么是 MinGW（windows-gnu）不是 MSVC：账号库的 SQLCipher 要现编 OpenSSL，而 OpenSSL 的 MSVC 构建
    非得用 Windows 上的 nmake.exe；MinGW 这条走普通 make，Mac 上就能编。
    WebView2Loader.dll 会被 tauri 一起打进安装包（gnu 版要动态加载它）。
    """
    missing = windows_tools_missing()
    if missing:
        print("\n⚠ 这台 Mac 还不能打 Windows 包，先装：")
        for m in missing:
            print(f"    {m}")
        return False
    print("\n▶ 构建 Windows 安装包（MinGW 交叉编译；第一次要现编 OpenSSL，几分钟）")
    # 旧产物先清掉：这一步要是失败了，别让上一次同版本号的旧包被当成这次的发出去
    shutil.rmtree(WIN_BUNDLE, ignore_errors=True)
    env = {
        **os.environ,
        "TAURI_SIGNING_PRIVATE_KEY": KEY_PATH.read_text().strip(),
        "TAURI_SIGNING_PRIVATE_KEY_PASSWORD": "",
        # makensis 在 locale 为空的环境里（比如从 IDE / 别的程序里调起来）会直接 bad_alloc 崩掉
        "LANG": "en_US.UTF-8",
        "LC_ALL": "en_US.UTF-8",
    }
    r = subprocess.run(
        ["pnpm", "tauri", "build", "--target", WIN_TARGET, "--bundles", "nsis"],
        cwd=ROOT, env=env,
    )
    win = find_windows(read_version())
    if r.returncode or not win:
        print("\n⚠ Windows 包没打出来，看上面的报错")
        return False
    print(f"\n✓ Windows 包：{win[0].name}（+ 签名）")
    return True


def ago(path: Path):
    mins = (time.time() - path.stat().st_mtime) / 60
    if mins < 1:
        return "刚刚"
    if mins < 60:
        return f"{int(mins)} 分钟前"
    if mins < 60 * 24:
        return f"{int(mins / 60)} 小时前"
    return f"{int(mins / 60 / 24)} 天前"


def ask(prompt, default=""):
    try:
        return input(prompt).strip()
    except (EOFError, KeyboardInterrupt):
        print()
        sys.exit(0)


def bump_patch():
    """发布成功后把修订号 +1，让工作区直接停在"下一个版本"。

    不这么做的话，下次发版忘了调版本号，老客户端查更新会一直显示「已是最新」——
    发了等于没发，而且不报错，最难查。
    """
    cur = read_version()
    major, minor, patch = (int(x) for x in cur.split("."))
    nxt = f"{major}.{minor}.{patch + 1}"
    write_version(nxt)
    return cur, nxt


def artifact_version():
    """从产物文件名里抠出版本号，用来判断它跟当前配置对不对得上。"""
    art = find_artifacts()
    if not art:
        return None
    m = re.search(r"_(\d+\.\d+\.\d+)_", art[0].name)
    return m.group(1) if m else None


def bump_version():
    cur = read_version()
    print(f"\n当前版本 {cur}。在线更新靠版本号比大小，发新版必须比这个高，")
    print("否则老客户端查更新会一直显示「已是最新」。")
    while True:
        v = ask(f"新版本号（直接回车放弃修改）：")
        if not v:
            return
        if not re.fullmatch(r"\d+\.\d+\.\d+", v):
            print("  格式要是 x.y.z，比如 0.1.1")
            continue
        if tuple(map(int, v.split("."))) <= tuple(map(int, cur.split("."))):
            print(f"  必须高于当前的 {cur}，否则更新推不出去")
            continue
        write_version(v)
        print(f"  已改成 {v}")
        return


LSREGISTER = (
    "/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/"
    "LaunchServices.framework/Versions/A/Support/lsregister"
)


def unmount_stale_volumes():
    """卸掉上次构建残留的磁盘映像。

    `bundle_dmg.sh` 靠 AppleScript 指挥 Finder 摆图标，残留的同名卷会让它失败。
    """
    out = subprocess.run(["hdiutil", "info"], capture_output=True, text=True).stdout
    for line in out.splitlines():
        parts = line.split()
        if not parts:
            continue
        mount = parts[-1]
        if mount.startswith("/Volumes/") and ("dmg." in mount or "Switchboard" in mount):
            print(f"  卸载残留卷 {mount}")
            subprocess.run(["hdiutil", "detach", parts[0], "-force"], capture_output=True)


def clean_launch_services():
    """把构建产物和临时卷从 LaunchServices 注册表里注销。

    每次打 DMG 都会临时挂一个卷，里面那个 Switchboard.app 会被注册进去；
    卷卸载了条目还留着。攒几次之后**启动台里就会冒出好几个 Switchboard**，
    看着像装了很多份，其实只有 /Applications 那一个是真的。
    这里只注销非 /Applications 的条目，不动文件本身。
    """
    if not Path(LSREGISTER).exists():
        return
    dump = subprocess.run([LSREGISTER, "-dump"], capture_output=True, text=True).stdout
    ghosts = {
        m
        for m in re.findall(r"/[^\s]*Switchboard\.app", dump)
        if not m.startswith("/Applications/")
    }
    if not ghosts:
        return
    for g in sorted(ghosts):
        subprocess.run([LSREGISTER, "-u", g], capture_output=True)
    print(f"  已从启动台注销 {len(ghosts)} 个构建产物/临时卷的残留条目")
    subprocess.run(["killall", "Dock"], capture_output=True)


def run_tauri_build():
    """跑 tauri build，同时把输出打到屏幕和缓冲区（要拿输出判断失败原因）。"""
    env = {
        **os.environ,
        "TAURI_SIGNING_PRIVATE_KEY": KEY_PATH.read_text().strip(),
        "TAURI_SIGNING_PRIVATE_KEY_PASSWORD": "",
    }
    proc = subprocess.Popen(
        ["pnpm", "tauri", "build", "--target", "universal-apple-darwin"],
        cwd=ROOT, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
    )
    lines = []
    for line in proc.stdout:
        print(line, end="")
        lines.append(line)
    proc.wait()
    return proc.returncode, "".join(lines)


def plain_dmg():
    """兜底：不靠 Finder，自己用 hdiutil 出一个朴素 DMG。

    代价是没有自定义背景和图标位置——但有个能装的包，比卡住不能发版强。
    """
    app = BUNDLE / "macos/Switchboard.app"
    if not app.is_dir():
        die(f"连 .app 都没有，不是 DMG 的问题：{app}")
    version = read_version()
    out = BUNDLE / "dmg" / f"Switchboard_{version}_universal.dmg"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.unlink(missing_ok=True)

    stage = Path(tempfile.mkdtemp()) / "Switchboard"
    stage.mkdir()
    subprocess.run(["cp", "-R", str(app), str(stage)], check=True)
    (stage / "Applications").symlink_to("/Applications")
    r = subprocess.run(
        ["hdiutil", "create", "-volname", "Switchboard", "-srcfolder", str(stage),
         "-ov", "-format", "UDZO", str(out)],
        capture_output=True, text=True,
    )
    if r.returncode:
        die(f"兜底 DMG 也失败了：{r.stderr[:300]}")
    print(f"  已用 hdiutil 生成朴素 DMG（无自定义背景）：{out.name}")


def build():
    print("\n▶ 构建 universal 包（两个架构，比单架构慢不少）")
    unmount_stale_volumes()
    code, out = run_tauri_build()
    if code == 0:
        clean_launch_services()
        return

    # DMG 那步是 AppleScript 指挥 Finder 摆图标，对时序敏感，会偶发失败
    # （报 -10006 / "Can't get disk (-1728)"）。不是代码问题，重试通常就过。
    dmg_failed = "bundle_dmg.sh" in out
    if not dmg_failed:
        die("构建失败，看上面的报错")

    print("\n⚠ DMG 那步失败了（它靠 Finder 摆图标，偶发）。清理残留卷后重试一次…")
    unmount_stale_volumes()
    time.sleep(3)
    code, out = run_tauri_build()
    if code == 0:
        clean_launch_services()
        return

    print("\n⚠ 仍然失败。改用 hdiutil 自己出 DMG，不走 Finder 那套。")
    unmount_stale_volumes()
    code, _ = run_tauri_build_app_only()
    if code:
        die("连 .app 都构建不出来，不是 DMG 的问题")
    plain_dmg()
    clean_launch_services()


def run_tauri_build_app_only():
    """只出 .app（跳过 DMG），给兜底路径用。updater 的 tar.gz 照样会生成。"""
    env = {
        **os.environ,
        "TAURI_SIGNING_PRIVATE_KEY": KEY_PATH.read_text().strip(),
        "TAURI_SIGNING_PRIVATE_KEY_PASSWORD": "",
    }
    r = subprocess.run(
        ["pnpm", "tauri", "build", "--target", "universal-apple-darwin", "--bundles", "app"],
        cwd=ROOT, env=env,
    )
    return r.returncode, ""


def get_token():
    token, just_typed = load_token()
    who = check_token(token)
    print(f"  Gitee 账号：{who}")
    if just_typed:
        if ask("  要记住这个令牌吗？以后就不用再输 [Y/n] ").lower() in ("", "y", "yes"):
            save_token(token)
    return token


def keep_other_platforms(cur, version, platforms):
    """这次没发的平台（比如只发了 macOS，清单里原来的 windows-x86_64）要留着，
    不然只发一个平台，另一个平台的客户端就收不到更新了。

    但**只在版本号相同时留**：清单只有一个顶层 version，把旧版 Windows 包挂在新版本号底下，
    Windows 客户端装完一看版本号还是旧的，下次又提示更新，没完没了。版本不同就去掉，等 Windows 发同版本再补。
    """
    try:
        old = json.loads(base64.b64decode(cur.get("content", "")).decode())
    except (ValueError, UnicodeDecodeError, AttributeError):
        return platforms
    others = {k: v for k, v in old.get("platforms", {}).items() if k not in platforms}
    if not others:
        return platforms
    if old.get("version") != version:
        print(f"  ⚠ 旧清单里的 {', '.join(others)} 是 {old.get('version')} 版，跟这次不同，已去掉；"
              f"等那个平台也发了 {version} 会补回来")
        return platforms
    print(f"  保留旧清单里的 {', '.join(others)}")
    return {**others, **platforms}


PLATFORM_NAMES = {"mac": "macOS", "win": "Windows"}


def describe(mac, win):
    """发行版说明 / 清单 notes 里写的平台"""
    return "、".join(n for n, on in (("macOS universal（Apple Silicon + Intel）", mac), ("Windows x64", win)) if on)


def publish(token, version, plats):
    """把选中的平台（plats ⊆ {"mac", "win"}）传进同一个发行版，并写进 latest.json"""
    art = find_artifacts() if "mac" in plats else None
    win = find_windows(version) if "win" in plats else None
    if "mac" in plats and not art:
        die(f"没找到 macOS 产物（或缺签名文件）。目录：{BUNDLE}")
    if "win" in plats and not win:
        die(f"没找到 {version} 的 Windows 安装包（或缺签名文件）。目录：{WIN_BUNDLE}")
    files = (list(art[:2]) if art else []) + ([win[0]] if win else [])
    tag = f"v{version}"

    platforms = {}
    # 1) 发行版：同 tag 已存在就复用，不重复创建（先发一个平台、再补另一个时就是这样）
    print(f"\n▶ 创建/复用发行版 {tag}")
    rel = api("GET", f"/repos/{OWNER}/{REPO}/releases/tags/{tag}", token=token)
    release_id = rel.get("id")
    if not release_id:
        rel = api("POST", f"/repos/{OWNER}/{REPO}/releases", token=token, body={
            "tag_name": tag,
            "name": f"Switchboard {tag}",
            "body": f"{describe(bool(art), bool(win))}。\n"
                    "macOS 下载 .dmg，Windows 下载 -setup.exe；App 内「设置 → 检查更新」可自动升级。",
            "target_commitish": BRANCH,
        })
        release_id = rel.get("id")
    if not release_id:
        die(f"发行版创建失败：{json.dumps(rel, ensure_ascii=False)[:400]}")
    print(f"  release id = {release_id}")

    # 2) 上传产物，回读真实下载地址（Gitee 的附件 URL 事先猜不出来）
    print("▶ 上传产物")
    urls = {}
    for f in files:
        res = upload_attachment(release_id, f, token)
        url = res.get("browser_download_url")
        if not url:
            die(f"上传 {f.name} 没拿到下载地址：{json.dumps(res, ensure_ascii=False)[:300]}")
        urls[f.name] = url
        print(f"  {f.name} → {url}")

    # 3) latest.json：更新端点读的就是它
    print("▶ 写 latest.json")
    cur = api("GET", f"/repos/{OWNER}/{REPO}/contents/latest.json",
              token=token, params=f"?ref={BRANCH}")
    sha = cur.get("sha") if isinstance(cur, dict) else None
    if art:
        signature = art[2].read_text().strip()
        platforms["darwin-aarch64"] = {"signature": signature, "url": urls[art[1].name]}
        platforms["darwin-x86_64"] = {"signature": signature, "url": urls[art[1].name]}
    if win:
        # Windows 在线更新装的就是 NSIS 安装包本身，签名是打包时用同一把私钥签的
        platforms["windows-x86_64"] = {"signature": win[1].read_text().strip(), "url": urls[win[0].name]}
    merged = keep_other_platforms(cur, version, platforms)
    manifest = {
        "version": version,
        # 按清单里最终有的平台写（只发一个平台时，另一个同版本的条目会被保留）
        "notes": describe(any(k.startswith("darwin") for k in merged), "windows-x86_64" in merged),
        "pub_date": datetime.datetime.now(datetime.timezone.utc)
        .isoformat(timespec="seconds").replace("+00:00", "Z"),
        "platforms": merged,
    }
    content = base64.b64encode(
        json.dumps(manifest, ensure_ascii=False, indent=2).encode()
    ).decode()
    payload = {"content": content, "message": f"release {tag}", "branch": BRANCH}
    if sha:
        payload["sha"] = sha
    res = api("PUT" if sha else "POST", f"/repos/{OWNER}/{REPO}/contents/latest.json",
              token=token, body=payload)
    if "_status" in res:
        die(f"写 latest.json 失败 HTTP {res['_status']}：{res['_body']}")

    print("\n✓ 发布完成")
    print(f"  发行版   ：https://gitee.com/{OWNER}/{REPO}/releases/tag/{tag}")
    print(f"  更新清单 ：https://gitee.com/{OWNER}/{REPO}/raw/{BRANCH}/latest.json")


def pick_platforms():
    """发哪几个平台。默认两个都发，也可以只发其中一个"""
    print("\n发布哪些平台？")
    print("  1) macOS + Windows（默认）")
    print("  2) 只发 macOS")
    print("  3) 只发 Windows")
    return {"1": {"mac", "win"}, "2": {"mac"}, "3": {"win"}}.get(ask("请选择 [1]：") or "1")


def after_publish(version, plats):
    """两个平台都发了就进位；只发了一个，要问一句——之后可能还要给这个版本补另一个平台"""
    if plats == {"mac", "win"}:
        old, nxt = bump_patch()
        print(f"\n▶ 版本号已自动进位：{old} → {nxt}")
        print("  下次改完代码直接跑本脚本选 1 就行，不用再手动改版本号。")
        return
    done = PLATFORM_NAMES[next(iter(plats))]
    other = PLATFORM_NAMES[({"mac", "win"} - plats).pop()]
    print(f"\n这次只发了 {done}。之后还要给 {version} 补发 {other} 的话，版本号先别进位；不补了就进位。")
    if ask("版本号进位吗？[Y/n] ").lower() in ("", "y", "yes"):
        old, nxt = bump_patch()
        print(f"▶ 版本号已进位：{old} → {nxt}")
        return
    print(f"  没进位，还是 {version}。补发时跑本脚本选 1 或 3 →「只发 {other}」，会传进同一个发行版。")
    print("  不补了的话，下次发版前一定要进位（选 4），不然老客户端查更新一直显示「已是最新」。")


def main():
    if not KEY_PATH.exists():
        die(f"找不到签名私钥 {KEY_PATH}。没有它签不出更新包，客户端装不上。")

    while True:
        version = read_version()
        art = find_artifacts()
        win = find_windows(version)
        print("\n" + "─" * 52)
        print(f"Switchboard 发布    版本 {version}  →  发行版 tag v{version}")
        av = artifact_version()
        stale = art is not None and av is not None and av != version
        if art:
            note = f"，属于 {av}，**与当前版本不符**" if stale else ""
            print(f"macOS   ：{art[0].name}（{ago(art[0])}构建，含签名{note}）")
        else:
            print("macOS   ：还没有产物")
        if win:
            print(f"Windows ：{win[0].name}（{ago(win[0])}构建，含签名）")
        elif windows_tools_missing():
            print("Windows ：这台 Mac 还没装交叉编译工具（装法见 docs/RELEASE.md「打包环境」）")
        else:
            print(f"Windows ：还没有 {version} 的安装包")
        # 能不能直接上传，按平台分别算：产物在、而且是当前版本
        ready = {"mac"} if art and not stale else set()
        ready |= {"win"} if win else set()
        print("─" * 52)
        print("  1) 发布          重新构建并上传（默认 macOS + Windows，也可以只发一个）")
        print("  2) 只构建        两个平台都出包，不上传")
        if ready:
            print(f"  3) 直接上传      复用已有产物（现在有：{'、'.join(PLATFORM_NAMES[p] for p in sorted(ready))}）")
        else:
            print(f"  3) 直接上传      不可用：没有 {version} 的产物，请选 1 或 2")
        print("  4) 改版本号")
        print("  0) 退出")
        choice = ask("请选择 [1]：") or "1"

        if choice == "0":
            return
        if choice == "4":
            bump_version()
            continue
        if choice == "2":
            build()
            build_windows()
            art = find_artifacts()
            if art:
                arch = subprocess.run(
                    ["lipo", "-archs", str(BUNDLE / "macos/Switchboard.app/Contents/MacOS/switchboard")],
                    capture_output=True, text=True).stdout.strip()
                print(f"\n✓ 构建完成：{art[0].name}（{arch}）")
                win = find_windows(version)
                if win:
                    print(f"            {win[0].name}")
                print("  要发布的话再跑一次，选「直接上传」（可以选发两个平台还是其中一个）")
            return
        if choice not in ("1", "3"):
            print("  没这个选项")
            continue

        plats = pick_platforms()
        if not plats:
            print("  没这个选项")
            continue
        if choice == "3" and not plats <= ready:
            lack = "、".join(PLATFORM_NAMES[p] for p in sorted(plats - ready))
            # 版本号发完自动进位了，旧产物配新 tag 会传出一个名实不符的包
            print(f"  {lack} 没有 {version} 的现成产物，直接传会名实不符或者没东西可传。请选 1 重新构建")
            continue
        if choice == "1" and "win" in plats and windows_tools_missing():
            print("  这台 Mac 还没装 Windows 交叉编译工具，打不了 Windows 包：")
            for m in windows_tools_missing():
                print(f"    {m}")
            print("  装好再来，或者这次选「只发 macOS」")
            continue

        # 令牌在构建**之前**验：构建要好几分钟，令牌错了才报会很难受
        print()
        token = get_token()
        if choice == "1":
            if "mac" in plats:
                build()
            if "win" in plats and not build_windows():
                die("Windows 包没打出来，什么都没发。想先只发 macOS 的话重跑，选「只发 macOS」")

        # 发布是往公开仓库推东西，动手前再确认一次
        art = find_artifacts() if "mac" in plats else None
        win = find_windows(version) if "win" in plats else None
        if ("mac" in plats and not art) or ("win" in plats and not win):
            die("构建后仍找不到产物")
        print(f"\n即将发布到 https://gitee.com/{OWNER}/{REPO}")
        print(f"  版本 v{version}")
        if art:
            print(f"  macOS  ：{art[0].name}")
            print(f"           {art[1].name}（+ 签名）")
        if win:
            print(f"  Windows：{win[0].name}（+ 签名）")
        if len(plats) == 1:
            other = PLATFORM_NAMES[({"mac", "win"} - plats).pop()]
            print(f"  只发 {PLATFORM_NAMES[next(iter(plats))]}：latest.json 里 {other} 的条目只在它也是 {version} 时保留；")
            print(f"           不是的话会被去掉，{other} 用户查更新会提示失败，直到 {other} 也发了 {version}")
        print("  并更新 latest.json —— 所有老客户端都会收到这个更新")
        if ask("确认发布？[y/N] ").lower() not in ("y", "yes"):
            print("  已取消")
            return
        publish(token, version, plats)
        after_publish(version, plats)
        return


if __name__ == "__main__":
    main()
