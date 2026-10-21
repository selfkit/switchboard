#!/usr/bin/env python3
"""把 Tauri 打 NSIS 包要用的 nsis_tauri_utils.dll 放进本地工具缓存。

为什么需要这一步：Tauri 的 bundler 自己用 ureq 从 GitHub 下载这个 DLL，
国内网络（尤其走代理 / 透明代理）下经常拿到被截断的响应，报
`io: unexpected end of file`，然后整包打不出来。curl 走同一套网络却稳得多，
所以这里先用 curl 下好、校验 SHA1，再让 tauri 直接命中缓存。

缓存位置由 `src-tauri/tauri.conf.json` 的 `bundle.useLocalToolsDir: true` 决定：
`src-tauri/target/.tauri/NSIS/Plugins/x86-unicode/additional/nsis_tauri_utils.dll`。

URL / SHA1 对应 `@tauri-apps/cli` 2.11.4（tauri-bundler 2.9.4）。升级 Tauri CLI 后
如果缓存被判成 mis-hashed，重新跑一次 release.py 即可；不行就更新下面两个常量。
"""
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

# nsis-tauri-utils v0.5.3
URL = os.environ.get(
    "SWITCHBOARD_NSIS_UTILS_URL",
    "https://github.com/tauri-apps/nsis-tauri-utils/releases/download/"
    "nsis_tauri_utils-v0.5.3/nsis_tauri_utils.dll",
)
SHA1 = os.environ.get("SWITCHBOARD_NSIS_UTILS_SHA1", "75197fee3c6a814fe035788d1c34ead39349b860").lower()
# 相对 NSIS 工具根目录；两边保持一致
REL = Path("Plugins/x86-unicode/additional/nsis_tauri_utils.dll")


def cache_path(root: Path) -> Path:
    return root / "src-tauri" / "target" / ".tauri" / "NSIS" / REL


def _sha1(path: Path) -> str:
    h = hashlib.sha1()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def _download(dest: Path) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp_name = tempfile.mkstemp(prefix="nsis_tauri_utils-", suffix=".dll")
    os.close(fd)
    tmp = Path(tmp_name)
    try:
        curl = shutil.which("curl")
        if curl:
            r = subprocess.run(
                [
                    curl, "-fL", "--retry", "4", "--retry-delay", "2",
                    "--connect-timeout", "20", "--max-time", "180",
                    "-o", str(tmp), URL,
                ]
            )
            if r.returncode:
                raise RuntimeError(f"curl 退出码 {r.returncode}")
        else:
            req = urllib.request.Request(URL, headers={"User-Agent": "switchboard-build"})
            with urllib.request.urlopen(req, timeout=180) as resp, tmp.open("wb") as out:
                shutil.copyfileobj(resp, out)
        got = _sha1(tmp)
        if got != SHA1:
            raise RuntimeError(f"SHA1 不对：{got}（期望 {SHA1}）")
        shutil.move(str(tmp), dest)
    finally:
        tmp.unlink(missing_ok=True)


def ensure(root: Path, quiet: bool = False) -> bool:
    """缓存里已有且哈希对就什么都不做；否则下载。失败返回 False 并打印手动步骤。"""
    dest = cache_path(root)
    if dest.exists() and _sha1(dest) == SHA1:
        return True
    if not quiet:
        print(f"\n▶ 准备 NSIS 插件缓存：{dest.relative_to(root)}")
    try:
        _download(dest)
    except Exception as e:  # noqa: BLE001 - 这里就是要兜住所有下载错误并给出人话
        print(f"✗ NSIS 插件没准备好：{e}")
        print("  手动做法：下载")
        print(f"    {URL}")
        print("  校验 SHA1 后放到")
        print(f"    {dest}")
        return False
    if not quiet:
        print("✓ NSIS 插件已就绪")
    return True


if __name__ == "__main__":
    root = Path(__file__).resolve().parent.parent
    sys.exit(0 if ensure(root) else 1)
