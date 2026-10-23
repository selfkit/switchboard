import { useEffect, useMemo, useRef, useState } from "react";
import * as api from "../api";
import type { Account, SessionInfo } from "../api";
import { Button, C, Dialog, ErrorLine, MOD, Pressable, R, inputStyle } from "../ui";

/** QuickSwitch.dc.html —— Cmd+K 面板 */
export function QuickSwitch({
  accounts,
  sessions,
  onClose,
  onPick,
}: {
  accounts: Account[];
  sessions: SessionInfo[];
  onClose: () => void;
  onPick: (a: Account) => void;
}) {
  const [q, setQ] = useState("");
  const [cursor, setCursor] = useState(0);
  const online = useMemo(() => new Set(sessions.map((s) => s.account_id)), [sessions]);

  const list = useMemo(() => {
    const kw = q.trim().toLowerCase();
    const match = accounts.filter((a) =>
      !kw ? true : [a.platform, a.related_app, a.username, a.remark, a.owner_type].some((s) => s.toLowerCase().includes(kw)),
    );
    // 已在线的排前面，对应原型里的两段分组
    return [...match.filter((a) => online.has(a.id)), ...match.filter((a) => !online.has(a.id))];
  }, [accounts, q, online]);

  useEffect(() => setCursor(0), [q]);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setCursor((c) => Math.min(c + 1, list.length - 1));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setCursor((c) => Math.max(c - 1, 0));
      } else if (e.key === "Enter" && list[cursor]) {
        e.preventDefault();
        onPick(list[cursor]);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [list, cursor, onPick]);

  const firstOffline = list.findIndex((a) => !online.has(a.id));

  return (
    <Dialog label="快捷切换账号" width={520} align="top" onClose={onClose} style={{ overflow: "hidden" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 10, padding: "14px 16px", borderBottom: `1px solid ${C.border}` }}>
        <span style={{ fontSize: 11, color: C.muted, border: `1px solid ${C.border}`, borderRadius: R.xs, padding: "2px 6px" }}>{MOD}K</span>
        <input
          autoFocus
          placeholder="输入平台名 / 应用名快速跳转"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          aria-label="搜索账号"
          style={{ flexGrow: 1, minWidth: 0, border: "none", background: "transparent", fontSize: 14, color: C.text }}
        />
      </div>
      <div style={{ maxHeight: 340, overflowY: "auto", padding: 8 }}>
        {list.length === 0 && <div style={{ padding: 16, fontSize: 13, color: C.muted }}>没有匹配的账号</div>}
        {list.map((a, i) => (
          <div key={a.id}>
            {i === 0 && online.has(a.id) && <Group text="当前已在线 · 直接切换" />}
            {i === firstOffline && <Group text="还没打开 · 切换即拉起页面" />}
            <Pressable
              onClick={() => onPick(a)}
              onMouseEnter={() => setCursor(i)}
              label={`${online.has(a.id) ? "切换" : "打开"} ${a.platform} ${a.related_app || a.username}`}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 10,
                padding: "10px 10px",
                borderRadius: R.md,
                background: i === cursor ? C.brandSoft : "transparent",
              }}
            >
              <div style={{ display: "flex", alignItems: "center", gap: 10, flexGrow: 1, minWidth: 0 }}>
                <div aria-hidden="true" style={{ width: 8, height: 8, borderRadius: "var(--sb-radius-pill)", background: online.has(a.id) ? C.success : C.border, flexShrink: 0 }} />
                <span style={{ fontSize: 13, color: C.text }}>{a.platform}</span>
                <span style={{ fontSize: 13, color: C.sub, flexGrow: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {a.related_app || a.username}
                  <span style={{ color: C.muted, fontSize: 12 }}> · {a.owner_type}</span>
                </span>
                <span style={{ fontSize: 11, color: C.muted }}>{online.has(a.id) ? "Enter 切换" : "Enter 打开"}</span>
              </div>
            </Pressable>
          </div>
        ))}
      </div>
      <div style={{ display: "flex", gap: 14, padding: "10px 16px", borderTop: `1px solid ${C.border}`, fontSize: 11, color: C.muted }}>
        <span>↑↓ 选择</span>
        <span>Enter 切换</span>
        <span>Esc 取消</span>
      </div>
    </Dialog>
  );
}

/**
 * 复制明文前的主密码闸门。
 *
 * 剪贴板是全机共享的，任何进程都能读；账号库再怎么加密，值一进剪贴板就裸了。
 * 所以往剪贴板放东西这件事单独要一次主密码。验过一次记 `COPY_GRACE_MS`，
 * 连着复制账号再复制密码不会被打断两回（宽限期在 App 里管，锁库即清空）。
 */
export function CopyGate({ label, onCancel, onPassed }: { label: string; onCancel: () => void; onPassed: () => void }) {
  const [pwd, setPwd] = useState("");
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);

  async function submit() {
    if (!pwd || busy) return;
    setBusy(true);
    setErr("");
    try {
      await api.verifyMasterPassword(pwd);
      setPwd("");
      onPassed();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog label={`复制${label}前验证主密码`} width={380} onClose={onCancel}>
      <div style={{ padding: 28, display: "flex", flexDirection: "column", gap: 14 }}>
        <div>
          <div style={{ fontSize: 17, color: C.text, fontWeight: 500 }}>复制{label}</div>
          <div style={{ fontSize: 12, color: C.muted, marginTop: 6, lineHeight: 1.7 }}>
            剪贴板是全机共享的，放进去的明文任何程序都读得到。验一次主密码后 5 分钟内的复制不再询问；
            复制的内容 30 秒后自动从剪贴板清掉。
          </div>
        </div>
        <input
          type="password"
          autoFocus
          value={pwd}
          placeholder="主密码"
          onChange={(e) => setPwd(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && void submit()}
          style={inputStyle}
        />
        <ErrorLine text={err} />
        <div style={{ display: "flex", gap: 10, justifyContent: "flex-end" }}>
          <Button size="md" onClick={onCancel}>取消</Button>
          <Button size="md" kind="primary" onClick={() => void submit()} disabled={!pwd || busy} loading={busy}>
            {busy ? "校验中…" : "验证并复制"}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

function Group({ text }: { text: string }) {
  return <div style={{ fontSize: 11, color: C.muted, padding: "10px 10px 6px" }}>{text}</div>;
}

/** TotpCode.dc.html —— 本地动态验证码，每秒刷新倒计时 */
export function TotpCode({
  account,
  onClose,
  onCopy,
}: {
  account: Account;
  onClose: () => void;
  /** 走 App 的主密码闸门；三态是为了把"用户取消"和"复制失败"分开 */
  onCopy: (label: string, text: string) => Promise<"done" | "error" | "cancel">;
}) {
  const [state, setState] = useState<{ code: string; remaining: number } | null>(null);
  const [err, setErr] = useState("");
  const [copied, setCopied] = useState("");
  const timer = useRef<number>();
  const copy = () => {
    if (!state) return;
    void onCopy("验证码", state.code).then((r) => {
      // 验完闸门会把这个框放回来，所以这里说得出话；取消就什么都不说
      if (r !== "cancel") setCopied(r === "done" ? "已复制到剪贴板" : "复制失败，请检查剪贴板权限");
    });
  };

  useEffect(() => {
    const tick = () =>
      api
        .totpCode(account.id)
        .then(setState)
        .catch((e) => setErr(String(e)));
    tick();
    timer.current = window.setInterval(tick, 1000);
    return () => window.clearInterval(timer.current);
  }, [account.id]);

  return (
    <Dialog label={`${account.platform} 本地验证码`} width={380} onClose={onClose}>
      <div style={{ padding: 28, display: "flex", flexDirection: "column", gap: 18, alignItems: "center" }}>
        <div style={{ textAlign: "center" }}>
          <div style={{ fontSize: 15, color: C.text }}>
            {account.platform} · {account.related_app || account.username}
          </div>
          <div style={{ fontSize: 12, color: C.muted, marginTop: 4 }}>本地动态验证码</div>
        </div>

        {err ? (
          <div role="alert" style={{ fontSize: 13, color: C.danger, textAlign: "center" }}>{err}</div>
        ) : (
          <>
            <button
              type="button"
              onClick={copy}
              title="点击复制"
              aria-label="复制验证码"
              style={{
                padding: "2px 4px",
                border: 0,
                background: "transparent",
                fontSize: 40,
                letterSpacing: 6,
                color: C.text,
                fontFamily: "ui-monospace, monospace",
                cursor: "pointer",
                borderRadius: R.md,
              }}
            >
              {state ? `${state.code.slice(0, 3)} ${state.code.slice(3)}` : "— — —"}
            </button>
            <div style={{ width: "100%", display: "flex", flexDirection: "column", gap: 8, alignItems: "center" }}>
              <div style={{ width: "100%", height: 4, borderRadius: "var(--sb-radius-pill)", background: C.bg, overflow: "hidden" }}>
                <div style={{ width: `${((state?.remaining ?? 0) / 30) * 100}%`, height: "100%", background: C.brand, transition: "width 1s linear" }} />
              </div>
              <div role="status" style={{ fontSize: 12, color: copied ? (copied.startsWith("已复制") ? C.success : C.danger) : C.muted }}>
                {copied || (state ? `${state.remaining} 秒后刷新` : "生成中…")}
              </div>
            </div>
          </>
        )}

        <div style={{ display: "flex", gap: 10 }}>
          {!err && (
            <Button size="md" kind="primary" onClick={copy} disabled={!state}>
              复制
            </Button>
          )}
          <Button size="md" onClick={onClose}>返回</Button>
        </div>
      </div>
    </Dialog>
  );
}
