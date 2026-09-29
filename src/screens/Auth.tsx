import { useEffect, useState, type ReactNode } from "react";
import * as api from "../api";
import { Button, C, ErrorLine, IS_MAC, copyText } from "../ui";

/** Register.dc.html / Unlock.dc.html 共用的居中布局 */
function Shell({ title, sub, children, foot }: { title: string; sub: string; children: ReactNode; foot: ReactNode }) {
  return (
    <div style={{ flexGrow: 1, minWidth: 0, display: "flex", flexDirection: "column" }}>
      <div
        style={{
          flexGrow: 1,
          minHeight: 0,
          overflowY: "auto",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          padding: "24px 0 48px",
        }}
      >
        <div style={{ width: 380, display: "flex", flexDirection: "column", gap: 24, alignItems: "center" }}>
          <div style={{ display: "flex", flexDirection: "column", gap: 8, alignItems: "center", textAlign: "center" }}>
            <h1 style={{ margin: 0, fontWeight: 500, fontSize: 30, color: C.text }}>{title}</h1>
            <div style={{ fontSize: 13, color: C.sub }}>{sub}</div>
          </div>
          <div style={{ width: "100%", display: "flex", flexDirection: "column", gap: 14 }}>{children}</div>
          {foot}
        </div>
      </div>
    </div>
  );
}

const bigInput: React.CSSProperties = {
  width: "100%",
  boxSizing: "border-box",
  padding: "12px 14px",
  borderRadius: 8,
  border: `1px solid ${C.borderStrong}`,
  background: C.surface,
  fontSize: 15,
  color: C.text,
};

const bigButton: React.CSSProperties = {
  width: "100%",
  boxSizing: "border-box",
  padding: "12px 14px",
  borderRadius: 8,
  border: "none",
  background: C.brand,
  color: C.onBrand,
  fontSize: 15,
  fontWeight: 500,
};

function Labeled({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      <label style={{ fontSize: 12, color: C.sub }}>{label}</label>
      {children}
    </div>
  );
}

type DemoProps = { onDemo: () => void; onTour: (record: boolean) => void };

/** 演示入口：换到一套假账号，登录页全在本机模拟、不连外网，真实账号库不碰。锁定即退出 */
function DemoEntry({ onDemo, onTour }: DemoProps) {
  const [record, setRecord] = useState(false);
  const link = { fontSize: 12, color: C.muted, cursor: "pointer", textDecoration: "underline" } as const;
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6, alignItems: "center" }}>
      <div style={{ display: "flex", gap: 16 }}>
        <span onClick={onDemo} title="假账号 + 本机模拟的登录页，不连外网；锁定即退出演示" style={link}>
          进入演示模式
        </span>
        <span onClick={() => onTour(record)} title="从全新的假账号库开始，自动把主要功能走一遍，约 30 秒" style={link}>
          ▶ 一键自动演示
        </span>
      </div>
      {/* 录屏用的是 macOS 自带的 screencapture，别的系统不显示这个选项 */}
      {IS_MAC && (
        <label style={{ fontSize: 11, color: C.muted, display: "flex", alignItems: "center", gap: 4, cursor: "pointer" }}>
          <input type="checkbox" checked={record} onChange={(e) => setRecord(e.target.checked)} />
          自动演示时录屏（只录本窗口，存到「影片」）
        </label>
      )}
      <div style={{ fontSize: 11, color: C.muted }}>演示全是假数据，不碰真实账号库</div>
    </div>
  );
}

export function Register({ onDone, onDemo, onTour }: { onDone: () => void } & DemoProps) {
  const [username, setUsername] = useState("");
  const [pwd, setPwd] = useState("");
  const [pwd2, setPwd2] = useState("");
  const [err, setErr] = useState("");

  async function submit() {
    if (pwd.length < api.MIN_PASSWORD) return setErr(`主密码至少 ${api.MIN_PASSWORD} 位`);
    if (pwd !== pwd2) return setErr("两次输入的主密码不一致");
    try {
      await api.register(username, pwd);
      onDone();
    } catch (e) {
      setErr(String(e));
    }
  }

  return (
    <Shell
      title="创建你的账号"
      sub="首次使用，先建立你自己在本机的登录身份"
      foot={
        <div style={{ display: "flex", flexDirection: "column", gap: 10, alignItems: "center" }}>
          <div style={{ fontSize: 12, color: C.muted, textAlign: "center", lineHeight: 1.6 }}>
            主密码只保存在本机，忘记后无法找回
            <br />
            也无法解密已有账号库，请妥善保管
          </div>
          <DemoEntry onDemo={onDemo} onTour={onTour} />
        </div>
      }
    >
      <Labeled label="用户名 / 邮箱">
        <input style={bigInput} placeholder="tan@example.com" value={username} onChange={(e) => setUsername(e.target.value)} />
      </Labeled>
      <Labeled label="设置主密码">
        <input style={bigInput} type="password" placeholder={`至少 ${api.MIN_PASSWORD} 位`} value={pwd} onChange={(e) => setPwd(e.target.value)} />
      </Labeled>
      <Labeled label="确认主密码">
        <input
          style={bigInput}
          type="password"
          placeholder="再次输入"
          value={pwd2}
          onChange={(e) => setPwd2(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && submit()}
        />
      </Labeled>
      <ErrorLine text={err} />
      <button style={bigButton} onClick={submit}>
        创建账号并开始使用
      </button>
    </Shell>
  );
}

export function Unlock({ onDone, onDemo, onTour, reason }: { onDone: () => void; reason?: string } & DemoProps) {
  const [pwd, setPwd] = useState("");
  const [err, setErr] = useState("");
  const [owner, setOwner] = useState("");
  const [hint, setHint] = useState<{ data_dir: string; command: string } | null>(null);
  const [blocked, setBlocked] = useState<{ verdict: api.Verdict; mode: api.RegionMode } | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    // 不吞异常：读不到身份是小事，但静默失败会让人以为是界面没写
    api.currentOwner().then(setOwner).catch((e) => setErr(String(e)));
  }, []);

  const showReset = () => (hint ? setHint(null) : api.resetHint().then(setHint).catch((e) => setErr(String(e))));

  /** 调用处一律写 `() => submit()`：直接 onClick={submit} 会把点击事件当成 force 传进来，等于绕过区域检测 */
  async function submit(force = false) {
    if (busy) return;
    setBusy(true);
    setErr("");
    try {
      const result = await api.unlock(pwd, force);
      if (result.blocked) {
        setBlocked({ verdict: result.blocked, mode: result.mode });
        return;
      }
      setBlocked(null);
      onDone();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Shell
      title="登录 Switchboard"
      sub="这是你系统里的账号，管理阿里云 / 腾讯云等外部账号库"
      foot={
        <div style={{ display: "flex", flexDirection: "column", gap: 10, alignItems: "center", width: "100%" }}>
          <div style={{ fontSize: 12, color: C.muted }}>整库由主密码加密，密钥从不上传</div>
          <span onClick={showReset} style={{ fontSize: 12, color: C.brand, cursor: "pointer" }}>
            忘记主密码？
          </span>
          {hint && <ResetPanel hint={hint} />}
          <DemoEntry onDemo={onDemo} onTour={onTour} />
        </div>
      }
    >
      {reason && (
        <div style={{ padding: "10px 14px", borderRadius: 8, background: C.brandSoft, color: C.brand, fontSize: 12, lineHeight: 1.6 }}>
          {reason}
        </div>
      )}
      {owner && (
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 10,
            padding: "10px 14px",
            background: C.surface,
            border: `1px solid ${C.border}`,
            borderRadius: 8,
          }}
        >
          <div
            style={{
              width: 28,
              height: 28,
              borderRadius: 999,
              background: C.brandSoft,
              color: C.brand,
              fontSize: 13,
              fontWeight: 600,
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              flexShrink: 0,
            }}
          >
            {[...owner][0]?.toUpperCase()}
          </div>
          <div style={{ display: "flex", flexDirection: "column", gap: 2, minWidth: 0, flexGrow: 1 }}>
            <span style={{ fontSize: 11, color: C.muted }}>当前账号</span>
            <span style={{ fontSize: 13, color: C.text, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{owner}</span>
          </div>
          <span onClick={showReset} style={{ fontSize: 12, color: C.brand, cursor: "pointer", flexShrink: 0 }}>
            不是你？
          </span>
        </div>
      )}

      <Labeled label="主密码">
        <input
          style={bigInput}
          type="password"
          placeholder="输入主密码"
          autoFocus
          value={pwd}
          onChange={(e) => setPwd(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && submit()}
        />
      </Labeled>
      {blocked && (
        <div style={{ padding: "12px 14px", borderRadius: 8, background: "rgba(227,77,89,0.08)", color: C.danger, fontSize: 12, lineHeight: 1.7 }}>
          <b>
            {blocked.verdict.origin.kind === "mainland" ? "检测到分流代理 / VPN" : "检测到境外网络"}
            {blocked.mode === "block" && "，已拦截解锁"}
          </b>
          <div>探测出口：{api.regionLabel(blocked.verdict)}{blocked.verdict.ip && `（${blocked.verdict.ip}）`}</div>
          <div>请关闭代理 / VPN，或切换到未检测到分流隧道的中国大陆网络后重新检测。</div>
          <div style={{ display: "flex", alignItems: "center", gap: 12, marginTop: 8 }}>
            <button type="button" onClick={() => submit()} disabled={busy} style={{ padding: "5px 10px", cursor: busy ? "default" : "pointer" }}>
              {busy ? "检测中…" : "重新检测并解锁"}
            </button>
            {/* 逃生口：误判时（公司专线出口在境外）没有它就解不开库，连检测模式都改不了 */}
            <span
              onClick={() => !busy && submit(true)}
              title="只是在本机读账号库；之后打开每个账号页仍会再检测"
              style={{ fontSize: 12, color: blocked.mode === "warn" ? C.brand : C.muted, cursor: busy ? "default" : "pointer" }}
            >
              {blocked.mode === "warn" ? "继续解锁" : "仍要解锁（不记住）"}
            </span>
          </div>
        </div>
      )}
      <ErrorLine text={err} />
      <button style={{ ...bigButton, opacity: blocked || busy ? 0.5 : 1, cursor: blocked || busy ? "not-allowed" : "pointer" }} onClick={() => submit()} disabled={!!blocked || busy}>
        {busy ? "检测中…" : "登录"}
      </button>
    </Shell>
  );
}

/**
 * 主密码就是整库的加密密钥，忘了就是真找不回来了——没有后门可以"重置成新密码"。
 * 所以这里不给按钮，只给一条要在数据目录里手敲的命令：它把旧库改名归档、让 App
 * 回到"创建主账号"，代价是旧账号数据从此打不开。故意让这一步费点劲。
 */
function ResetPanel({ hint }: { hint: { data_dir: string; command: string } }) {
  // null = 还没点；false = 复制失败（WKWebView 里剪贴板可能被拒，见 ui.tsx 的 copyText）
  const [copied, setCopied] = useState<boolean | null>(null);
  // Windows 上给的是 PowerShell 命令（见 lib.rs 的 reset_hint），cmd 里跑不了
  const shell = IS_MAC ? "终端" : " PowerShell ";
  return (
    <div style={{ width: "100%", background: C.surface, border: `1px solid ${C.border}`, borderRadius: 10, padding: 16, display: "flex", flexDirection: "column", gap: 10 }}>
      <div style={{ fontSize: 13, color: C.text }}>换个账号 / 忘了主密码</div>
      <div style={{ fontSize: 12, color: C.sub, lineHeight: 1.7 }}>
        主密码找不回来，也没有办法重置——它本身就是解密账号库的钥匙。
        继续的唯一办法是<b>放弃当前这个库、清空重来</b>，里面存的所有账号都会打不开。
        为了不让人手滑点掉整个库，这一步没有做成按钮，需要复制下面的命令，
        到{shell}里粘贴执行：
      </div>
      <div style={{ display: "flex", gap: 8, alignItems: "stretch" }}>
        <code
          style={{
            flexGrow: 1,
            minWidth: 0,
            background: C.bg,
            borderRadius: 7,
            padding: "10px 12px",
            fontSize: 11,
            color: C.sub,
            wordBreak: "break-all",
            fontFamily: "ui-monospace, monospace",
          }}
        >
          {hint.command}
        </code>
        <Button onClick={() => void copyText(hint.command).then(setCopied)} style={{ flexShrink: 0 }}>
          {copied ? "已复制" : "复制"}
        </Button>
      </div>
      <div style={{ fontSize: 11, color: copied === false ? C.danger : C.muted }}>
        {copied === true ? `去${shell}里粘贴执行，然后重开 App` : copied === false ? "复制失败，请手动选中上面的命令复制" : `复制后到${shell}粘贴执行`}
      </div>
    </div>
  );
}
