import { useEffect, useRef, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { open } from "@tauri-apps/plugin-dialog";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { RELEASE_PAGE } from "./DownloadGate";
import { RegionSettings } from "./RegionGuard";
import * as api from "../api";
import { Button, C, Card, ErrorLine, Field, inputStyle } from "../ui";
import { NavItem, SideGroup, SubPage, type SubPageShell } from "./Layout";

type SettingsTab = "region" | "update" | "download" | "backup" | "password";
const settingsTabs: { id: SettingsTab; label: string }[] = [
  { id: "region", label: "网络检测" },
  { id: "update", label: "版本更新" },
  { id: "download", label: "下载" },
  { id: "backup", label: "备份恢复" },
  { id: "password", label: "主密码" },
];

/** 下载存到哪：默认系统「下载」目录，可以换个文件夹，也可以每次下载完都问 */
function DownloadSettings() {
  const [s, setS] = useState<api.DownloadSettings | null>(null);
  const [err, setErr] = useState("");
  useEffect(() => {
    api.getDownloadSettings().then(setS).catch((e) => setErr(String(e)));
  }, []);

  async function apply(dir: string | null, ask: boolean) {
    setErr("");
    try {
      await api.setDownloadSettings(dir, ask);
      setS(await api.getDownloadSettings());
    } catch (e) {
      setErr(String(e));
    }
  }

  async function pickDir() {
    if (!s) return;
    const dir = await open({ directory: true, defaultPath: s.dir, title: "选择下载文件夹" });
    if (typeof dir === "string") await apply(dir, s.ask);
  }

  return (
    <Card style={{ display: "flex", flexDirection: "column", gap: 10 }}>
      <div style={{ fontSize: 15, color: C.text }}>下载位置</div>
      <div style={{ fontSize: 13, color: C.sub, lineHeight: 1.7 }}>
        账号页面里下载的文件存到这里，重名不覆盖（自动加 -2、-3），导出的加密备份也存这里。
        选的文件夹不在了（被删、改名、外接盘拔了）会自动退回系统「下载」目录。
      </div>
      {s && (
        <>
          <div style={{ background: C.bg, borderRadius: 8, padding: "10px 12px", fontSize: 12, color: C.sub, wordBreak: "break-all", fontFamily: "ui-monospace, monospace" }}>
            {s.dir}
            {!s.custom && !s.demo && <span style={{ color: C.muted, fontFamily: "inherit" }}>（系统「下载」目录）</span>}
          </div>
          <div style={{ display: "flex", gap: 10 }}>
            <Button onClick={() => void pickDir()} disabled={s.demo}>
              更改文件夹…
            </Button>
            {s.custom && <Button onClick={() => void apply(null, s.ask)}>恢复默认</Button>}
          </div>
          <label style={{ fontSize: 13, color: C.sub, display: "flex", alignItems: "center", gap: 8, marginTop: 4, cursor: s.demo ? "default" : "pointer" }}>
            <input type="checkbox" checked={s.ask} disabled={s.demo} onChange={(e) => void apply(s.custom ? s.dir : null, e.target.checked)} />
            每次下载都问保存到哪
          </label>
          <div style={{ fontSize: 11, color: C.muted, lineHeight: 1.6 }}>
            打开后，每个文件下完会弹出保存框，选好位置就挪过去；点取消就留在上面这个文件夹里。
          </div>
          {s.demo && <div style={{ fontSize: 12, color: C.muted }}>演示模式的下载固定存在演示目录里，免得演示时露出真实的文件夹。</div>}
        </>
      )}
      <ErrorLine text={err} />
    </Card>
  );
}

/** Settings.dc.html —— 备份导出导入 + 修改主密码 */
export default function Settings({ shell, onLocked, onWiped, onRegionChanged, onAutoLockChanged }: {
  shell: SubPageShell;
  onLocked: () => void;
  onWiped: () => void;
  onRegionChanged: (s: api.RegionState) => void;
  /** 改完立刻反映到 App 里正跑着的计时器，不用重新登录才生效 */
  onAutoLockChanged: (secs: number) => void;
}) {
  const [tab, setTab] = useState<SettingsTab>("region");
  const rootRef = useRef<HTMLDivElement>(null);
  const [msg, setMsg] = useState("");
  const [err, setErr] = useState("");
  const [exported, setExported] = useState("");

  // 导入要先选文件、再输那份备份当时的主密码。这里必须自己画输入框：
  // macOS 的 WKWebView 下 wry 没实现 JS 弹窗，window.prompt/alert 是静默失效的。
  // current 是当前账号库的主密码：整库替换跟清空一个门槛，后端会校验
  const [importing, setImporting] = useState<{ path: string; password: string; current: string } | null>(null);

  // 更新：只认 tauri.conf.json 里那把公钥签过的包，签名对不上直接拒装
  const [upd, setUpd] = useState<{ state: "idle" | "checking" | "none" | "found" | "downloading"; version?: string; notes?: string }>({
    state: "idle",
  });

  const [cur, setCur] = useState("");
  const [next, setNext] = useState("");
  const [next2, setNext2] = useState("");
  const [showPwd, setShowPwd] = useState(false);
  const [wiping, setWiping] = useState(false);
  const [wipePwd, setWipePwd] = useState("");
  const [version, setVersion] = useState("");
  const [autoLock, setAutoLock] = useState(0);
  const [savingAutoLock, setSavingAutoLock] = useState(false);

  useEffect(() => {
    rootRef.current?.parentElement?.scrollTo({ top: 0 });
  }, [tab]);

  useEffect(() => {
    void getVersion().then(setVersion);
  }, []);

  useEffect(() => {
    api.getAutoLockSecs().then(setAutoLock).catch((e) => setErr(String(e)));
  }, []);

  async function pickAutoLock(secs: number) {
    if (savingAutoLock || autoLock === secs) return;
    setErr("");
    setSavingAutoLock(true);
    try {
      await api.setAutoLockSecs(secs);
      setAutoLock(secs);
      onAutoLockChanged(secs);
    } catch (e) {
      setErr(String(e));
    } finally {
      setSavingAutoLock(false);
    }
  }

  function reset(then: () => void) {
    setErr("");
    setMsg("");
    then();
  }

  async function doExport() {
    reset(async () => {
      try {
        setExported(await api.exportBackup());
      } catch (e) {
        setErr(String(e));
      }
    });
  }

  async function pickBackup() {
    setErr("");
    setMsg("");
    const picked = await open({ multiple: false, filters: [{ name: "Switchboard 备份", extensions: ["db"] }] });
    if (typeof picked !== "string") return;
    setImporting({ path: picked, password: "", current: "" });
  }

  async function doImport() {
    if (!importing || !importing.password || !importing.current) return;
    setErr("");
    try {
      const n = await api.importBackup(importing.path, importing.password, importing.current);
      setImporting(null);
      setMsg(`已导入 ${n} 条账号记录。账号库现在用的是这份备份的主密码，请用它重新登录。`);
      setTimeout(onLocked, 1200);
    } catch (e) {
      setErr(String(e));
    }
  }

  async function doChangePassword() {
    setErr("");
    setMsg("");
    if (next !== next2) return setErr("两次输入的新主密码不一致");
    try {
      await api.changeMasterPassword(cur, next);
      setCur("");
      setNext("");
      setNext2("");
      setShowPwd(false);
      setMsg("主密码已修改。旧的备份文件仍然绑定旧主密码，建议重新导出一份。");
    } catch (e) {
      setErr(String(e));
    }
  }

  async function doWipe() {
    reset(async () => {
      try {
        await api.wipeVault(wipePwd);
        setWiping(false);
        setWipePwd("");
        onWiped();
      } catch (e) {
        setErr(String(e));
      }
    });
  }

  async function checkUpdate() {
    setErr("");
    setMsg("");
    setUpd({ state: "checking" });
    try {
      const u = await check();
      if (!u) return setUpd({ state: "none" });
      setUpd({ state: "found", version: u.version, notes: u.body });
    } catch (e) {
      setUpd({ state: "idle" });
      setErr(`检查更新失败：${e}`);
    }
  }

  async function installUpdate() {
    setErr("");
    try {
      const u = await check();
      if (!u) return setUpd({ state: "none" });
      setUpd({ state: "downloading" });
      await u.downloadAndInstall();
      await relaunch();
    } catch (e) {
      setUpd({ state: "idle" });
      setErr(`更新失败：${e}`);
    }
  }

  return (
    <SubPage
      current="settings"
      {...shell}
      side={
        <SideGroup title="设置">
          {settingsTabs.map(({ id, label }) => (
            <NavItem key={id} label={label} active={tab === id} onClick={() => setTab(id)} />
          ))}
        </SideGroup>
      }
    >
    <div ref={rootRef} style={{ display: "flex", flexDirection: "column", gap: 20, maxWidth: 860 }}>
      <div>
        <div style={{ fontSize: 12, color: C.muted, marginBottom: 4 }}>设置</div>
        <h1 style={{ margin: 0, fontWeight: 500, fontSize: 24, color: C.text }}>{settingsTabs.find((t) => t.id === tab)?.label}</h1>
      </div>

      <div id="settings-panel-region" hidden={tab !== "region"}>
        <RegionSettings onChanged={onRegionChanged} />
      </div>

      <div id="settings-panel-update" hidden={tab !== "update"}>
      <Card style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        <div style={{ fontSize: 15, color: C.text }}>版本与更新</div>
        <div style={{ fontSize: 12, color: C.muted }}>当前版本 {version || "…"}</div>
        <div style={{ fontSize: 13, color: C.sub, lineHeight: 1.7 }}>
          更新包从 Gitee 发行版下载，安装前会使用内置公钥验证签名。
        </div>
        <div style={{ display: "flex", gap: 10, marginTop: 6, alignItems: "center", flexWrap: "wrap" }}>
          {upd.state === "found" ? (
            <Button kind="primary" onClick={installUpdate}>
              更新到 {upd.version}
            </Button>
          ) : (
            <Button onClick={checkUpdate} disabled={upd.state === "checking" || upd.state === "downloading"}>
              {upd.state === "checking" ? "检查中…" : upd.state === "downloading" ? "下载中…" : "检查更新"}
            </Button>
          )}
          {/* 主窗口没接新窗口请求，target=_blank 点了没反应，交给系统浏览器开 */}
          <a
            href={RELEASE_PAGE}
            onClick={(e) => {
              e.preventDefault();
              api.openInSystemBrowser(RELEASE_PAGE).catch((x) => setErr(String(x)));
            }}
            style={{ fontSize: 12, color: C.brand }}
          >
            打开发行版页面
          </a>
        </div>
        {upd.state === "none" && <div style={{ fontSize: 12, color: C.muted }}>已经是最新版本</div>}
        {upd.state === "found" && upd.notes && (
          <div style={{ fontSize: 12, color: C.sub, background: C.bg, borderRadius: 8, padding: "10px 12px", whiteSpace: "pre-wrap", lineHeight: 1.7 }}>
            {upd.notes}
          </div>
        )}
        {upd.state === "downloading" && <div style={{ fontSize: 12, color: C.muted }}>正在下载并安装，完成后会自动重启</div>}
      </Card>
      </div>

      <div id="settings-panel-download" hidden={tab !== "download"}>
        <DownloadSettings />
      </div>
      <div id="settings-panel-backup" hidden={tab !== "backup"}>
      <Card style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        <div style={{ fontSize: 15, color: C.text }}>备份与导出</div>
        <div style={{ fontSize: 13, color: C.sub, lineHeight: 1.7 }}>
          导出整个账号库，包含账号、密码、TOTP 密钥和平台配置。备份文件使用当前主密码加密；在其他设备导入时需要输入导出时的主密码。
          备份<b>不含</b>各账号的登录状态（cookie 快照），免得备份文件流出去就等于散发一堆能直接用的登录凭证；用备份恢复后各账号重新登录一次即可。
          导入时当前账号库会改名留作后路，这类旧库只保留最近 3 份。
        </div>
        <div style={{ display: "flex", gap: 10, marginTop: 6 }}>
          <Button kind="primary" onClick={doExport}>
            导出加密备份
          </Button>
          <Button onClick={pickBackup}>从备份导入</Button>
        </div>
      </Card>
      </div>

      <div id="settings-panel-password" hidden={tab !== "password"}>
      <Card style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        <div style={{ fontSize: 15, color: C.text }}>主密码</div>
        <div style={{ fontSize: 13, color: C.sub }}>修改后需要用新密码重新加密整个账号库</div>
        {!showPwd ? (
          <div>
            <Button onClick={() => setShowPwd(true)}>修改主密码</Button>
          </div>
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: 12, marginTop: 6 }}>
            <Field label="当前主密码">
              <input style={inputStyle} type="password" value={cur} onChange={(e) => setCur(e.target.value)} />
            </Field>
            <Field label={`新主密码（至少 ${api.MIN_PASSWORD} 位）`}>
              <input style={inputStyle} type="password" value={next} onChange={(e) => setNext(e.target.value)} />
            </Field>
            <Field label="确认新主密码">
              <input style={inputStyle} type="password" value={next2} onChange={(e) => setNext2(e.target.value)} />
            </Field>
            <div style={{ display: "flex", gap: 10 }}>
              <Button kind="primary" onClick={doChangePassword}>
                确认修改
              </Button>
              <Button onClick={() => setShowPwd(false)}>取消</Button>
            </div>
          </div>
        )}
      </Card>

      <Card style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        <div style={{ fontSize: 15, color: C.text }}>自动锁定</div>
        <div style={{ fontSize: 13, color: C.sub, lineHeight: 1.7 }}>
          账号库解锁后，账号列表（用户名、备注等）是直接显示的。长时间不操作就自动锁回登录页，
          离开电脑时更安心。
        </div>
        <div role="radiogroup" aria-label="自动锁定时长" style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
          {([
            [0, "关闭"],
            [300, "5 分钟"],
            [900, "15 分钟"],
            [1800, "30 分钟"],
            [3600, "1 小时"],
          ] as const).map(([secs, label]) => {
            const selected = autoLock === secs;
            return (
              <button
                key={secs}
                type="button"
                aria-pressed={selected}
                disabled={savingAutoLock}
                onClick={() => void pickAutoLock(secs)}
                style={{
                  padding: "6px 10px",
                  borderRadius: 6,
                  border: `1px solid ${selected ? C.brand : C.border}`,
                  background: selected ? C.brandSoft : C.white,
                  color: selected ? C.brand : C.sub,
                  fontSize: 12,
                  cursor: savingAutoLock ? "default" : "pointer",
                  opacity: savingAutoLock ? 0.65 : 1,
                }}
              >
                {label}
              </button>
            );
          })}
        </div>
        <div style={{ fontSize: 11, color: C.muted, lineHeight: 1.6 }}>
          点击、按键、滚动都算操作，包括在账号页面里填外部登录表单；真的完全不碰鼠标键盘才会被计时。
        </div>
      </Card>

      <Card style={{ display: "flex", flexDirection: "column", gap: 10, border: `1px solid ${C.danger}` }}>
        <div style={{ fontSize: 15, color: C.danger }}>清空所有账户数据</div>
        <div style={{ fontSize: 13, color: C.sub, lineHeight: 1.7 }}>
          本机的账号库整个作废（改名归档，不是直接删除；归档只保留最近 3 份），各账号留在本机的登录状态一并清掉，App 会回到"创建主账号"。<b>没有任何后悔药</b>，
          需要的话请先在"备份与恢复"里导出一份。
        </div>
        <div>
          <Button kind="danger" onClick={() => setWiping(true)}>
            清空所有账户数据
          </Button>
        </div>
      </Card>
      </div>

      <ErrorLine text={err} />
      {msg && <div style={{ fontSize: 13, color: C.brand, lineHeight: 1.7 }}>{msg}</div>}

      {importing && (
        <div
          onClick={() => setImporting(null)}
          style={{ position: "fixed", inset: 0, background: "rgba(0,0,0,0.35)", display: "flex", alignItems: "center", justifyContent: "center" }}
        >
          <div
            onClick={(e) => e.stopPropagation()}
            style={{ width: 460, background: C.white, borderRadius: 14, padding: 28, display: "flex", flexDirection: "column", gap: 14 }}
          >
            <div style={{ fontSize: 18, color: C.text }}>从备份导入</div>
            <div
              style={{ background: C.bg, borderRadius: 8, padding: "10px 12px", fontSize: 11, color: C.sub, wordBreak: "break-all", fontFamily: "ui-monospace, monospace" }}
            >
              {importing.path}
            </div>
            <Field label="这份备份当时用的主密码">
              <input
                style={inputStyle}
                type="password"
                autoFocus
                value={importing.password}
                onChange={(e) => setImporting({ ...importing, password: e.target.value })}
                onKeyDown={(e) => e.key === "Enter" && doImport()}
              />
            </Field>
            <Field label="当前账号库的主密码">
              <input
                style={inputStyle}
                type="password"
                value={importing.current}
                onChange={(e) => setImporting({ ...importing, current: e.target.value })}
                onKeyDown={(e) => e.key === "Enter" && doImport()}
              />
            </Field>
            <div style={{ fontSize: 12, color: C.muted, lineHeight: 1.7 }}>
              导入会<b>整个替换</b>当前账号库（当前这份会自动改名留在数据目录里，不会直接删），
              所以跟清空账号库一样要当前主密码。导入后主密码变成这份备份的密码，需要重新登录。
            </div>
            <ErrorLine text={err} />
            <div style={{ display: "flex", gap: 10, justifyContent: "flex-end" }}>
              <Button onClick={() => setImporting(null)}>取消</Button>
              <Button kind="primary" onClick={doImport} disabled={!importing.password || !importing.current}>
                确认导入
              </Button>
            </div>
          </div>
        </div>
      )}

      {wiping && (
        <div
          onClick={() => { setWiping(false); setWipePwd(""); }}
          style={{ position: "fixed", inset: 0, background: "rgba(0,0,0,0.35)", display: "flex", alignItems: "center", justifyContent: "center" }}
        >
          <div
            onClick={(e) => e.stopPropagation()}
            style={{ width: 420, background: C.white, borderRadius: 14, padding: 28, display: "flex", flexDirection: "column", gap: 14 }}
          >
            <div style={{ fontSize: 18, color: C.danger }}>确认清空所有账户数据？</div>
            <div style={{ fontSize: 12, color: C.muted, lineHeight: 1.7 }}>
              这会作废本机当前的账号库，所有账号、密码、TOTP 都将无法通过 App 访问。此操作不可撤销，请先确认已经备份。
              跟改主密码一个门槛，输入当前主密码以继续：
            </div>
            <input
              style={inputStyle}
              type="password"
              autoFocus
              placeholder="当前主密码"
              value={wipePwd}
              onChange={(e) => setWipePwd(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && wipePwd && doWipe()}
            />
            <ErrorLine text={err} />
            <div style={{ display: "flex", gap: 10, justifyContent: "flex-end" }}>
              <Button onClick={() => { setWiping(false); setWipePwd(""); }}>取消</Button>
              <Button kind="danger" onClick={doWipe} disabled={!wipePwd}>
                确认清空
              </Button>
            </div>
          </div>
        </div>
      )}

      {exported && (
        <div
          onClick={() => setExported("")}
          style={{ position: "fixed", inset: 0, background: "rgba(0,0,0,0.35)", display: "flex", alignItems: "center", justifyContent: "center" }}
        >
          <div
            onClick={(e) => e.stopPropagation()}
            style={{ width: 420, background: C.white, borderRadius: 14, padding: 28, display: "flex", flexDirection: "column", gap: 16 }}
          >
            <div style={{ fontSize: 18, color: C.text }}>备份已导出</div>
            <div
              style={{ background: C.bg, borderRadius: 8, padding: "12px 14px", fontSize: 12, color: C.sub, wordBreak: "break-all", fontFamily: "ui-monospace, monospace" }}
            >
              {exported}
            </div>
            <div style={{ fontSize: 12, color: C.muted, lineHeight: 1.6 }}>
              这份备份文件只能用当前的主密码解密。改了主密码之后，记得重新导出一份新的
            </div>
            <div style={{ display: "flex", justifyContent: "flex-end" }}>
              <Button kind="primary" onClick={() => setExported("")}>
                完成
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
    </SubPage>
  );
}
