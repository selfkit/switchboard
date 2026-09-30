import { useEffect, useState } from "react";
import * as api from "../api";
import type { RegionEndpoint, RegionMode, Verdict } from "../api";
import { Button, C, Card, ErrorLine, inputStyle } from "../ui";

/** 出口状态的配色：大陆=绿、境外=红、未知=灰 */
function tone(v: Verdict) {
  if (v.origin.kind === "mainland") return { color: C.success, flag: "🇨🇳" };
  if (v.origin.kind === "outside") return { color: C.danger, flag: "🌍" };
  return { color: C.muted, flag: "？" };
}

/** 顶栏常驻状态：出口属地和分流代理信号分开显示，避免把国内探测出口误认为代理已关闭。 */
export function RegionBadge({ state, onClick, busy }: { state: api.RegionState | null; onClick: () => void; busy: boolean }) {
  const v = state?.verdict;
  const t = v ? tone(v) : null;
  const mainlandWithTunnel = v?.origin.kind === "mainland" && v.tunnel;
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={busy}
      title="主动检测网络出口"
      style={{
        display: "flex",
        alignItems: "center",
        gap: 5,
        padding: "5px 9px",
        borderRadius: 7,
        fontSize: 11,
        whiteSpace: "nowrap",
        flexShrink: 0,
        border: `1px solid ${C.border}`,
        cursor: busy ? "default" : "pointer",
        color: mainlandWithTunnel ? C.warn : t?.color ?? C.sub,
        background: v?.origin.kind === "outside" ? "rgba(227,77,89,0.08)" : mainlandWithTunnel ? "rgba(179,107,0,0.08)" : C.surface,
      }}
    >
      <span>{state?.mode === "off" && v?.origin.kind === "unknown" ? "🌐" : t?.flag ?? "🌐"}</span>
      <span>{busy ? "检测中…" : state?.mode === "off" && v?.origin.kind === "unknown" ? "检测已关闭 · 手动检测" : mainlandWithTunnel ? "上次：分流代理 / VPN · 重新检测" : v ? `上次：${api.regionLabel(v)} · 重新检测` : "检测网络"}</span>
      {mainlandWithTunnel && <span title="检测到隧道在跑">⚠️</span>}
    </button>
  );
}

/**
 * 检测到境外出口或分流隧道时的拦截/确认框。
 * `block` 与 `warn` 的区别只在措辞和主按钮——机制一样，都给逃生口。
 */
export function RegionBlocked({
  verdict,
  mode,
  accountLabel,
  onCancel,
  onContinue,
  onSettings,
  onRecheck,
}: {
  verdict: Verdict;
  mode: RegionMode;
  accountLabel: string;
  onCancel: () => void;
  onContinue: () => void;
  onSettings: () => void;
  onRecheck: () => Promise<void>;
}) {
  const strict = mode === "block";
  const splitTunnel = verdict.origin.kind === "mainland" && verdict.tunnel;
  const heading = splitTunnel ? "检测到分流代理 / VPN" : "检测到境外网络";
  const [rechecking, setRechecking] = useState(false);
  return (
    <div
      onClick={onCancel}
      style={{ position: "fixed", inset: 0, background: "rgba(0,0,0,0.35)", display: "flex", alignItems: "center", justifyContent: "center", zIndex: 30 }}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{ width: 460, background: C.surface, borderRadius: 14, padding: 28, display: "flex", flexDirection: "column", gap: 14 }}
      >
        <div style={{ fontSize: 18, color: C.text }}>{heading}{strict && "，已拦截"}</div>

        <div style={{ background: C.bg, borderRadius: 8, padding: "12px 14px", display: "flex", flexDirection: "column", gap: 6, fontSize: 12, color: C.sub }}>
          <div>
            探测出口：<b style={{ color: splitTunnel ? C.warn : C.danger }}>{api.regionLabel(verdict)}</b>
            {verdict.ip && <span style={{ color: C.muted }}>（{verdict.ip}）</span>}
          </div>
          <div>要打开：{accountLabel}</div>
        </div>

        <div style={{ fontSize: 13, color: C.sub, lineHeight: 1.8 }}>
          {splitTunnel
            ? "国内探针仍走大陆出口，但外网探针可达。目标站点可能被代理规则分流到境外，无法仅凭这个大陆 IP 判断，已按网络异常处理。"
            : "阿里云日志等服务因政策原因不允许从中国大陆以外访问，强行打开可能触发账号风控。"}
          {verdict.proxy && (
            <>
              <br />
              检测到系统配置了代理。若你确认这个站走的是直连，可以继续。
            </>
          )}
        </div>

        <div style={{ display: "flex", gap: 10, justifyContent: "flex-end", alignItems: "center", flexWrap: "wrap" }}>
          <span onClick={onSettings} style={{ fontSize: 12, color: C.muted, cursor: "pointer" }}>
            去设置
          </span>
          <span
            onClick={async () => {
              // 关掉 VPN 后要能立刻重试——结论缓存 10 分钟，不给这个按钮
              // 用户就得干等或者绕去设置页
              setRechecking(true);
              await onRecheck();
              setRechecking(false);
            }}
            style={{ fontSize: 12, color: C.brand, cursor: "pointer", marginRight: "auto" }}
          >
            {rechecking ? "检测中…" : "关了代理？重新检测"}
          </span>
          {strict ? (
            <>
              <span onClick={onContinue} style={{ fontSize: 12, color: C.muted, cursor: "pointer" }}>
                仍要继续（30 分钟内不断开）
              </span>
              <Button kind="primary" onClick={onCancel}>
                取消
              </Button>
            </>
          ) : (
            <>
              <Button onClick={onCancel}>取消</Button>
              <Button kind="primary" onClick={onContinue}>
                继续打开
              </Button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

/**
 * 哨兵在运行中探到境外出口或分流隧道时弹的东西。两种模式共用一个壳：
 * - `block`：后端**已经把页面全断了**，这里只是通知 + 给恢复的路
 * - `warn`：还没动手，问用户要不要断
 *
 * 不给"关掉"按钮而只给明确的两条路，是因为这时候页面已经是空白的了，
 * 随手关掉弹层只会剩一片白板，用户不知道发生了什么。
 */
export function RegionAlarm({
  verdict,
  cut,
  onCut,
  onIgnore,
  onSettings,
  onRecheck,
}: {
  verdict: Verdict;
  cut: boolean;
  onCut: () => void;
  onIgnore: () => void;
  onSettings: () => void;
  onRecheck: () => Promise<boolean>;
}) {
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const splitTunnel = verdict.origin.kind === "mainland" && verdict.tunnel;
  const heading = splitTunnel ? "检测到分流代理 / VPN" : "网络变成境外";
  return (
    <div style={{ position: "fixed", inset: 0, background: "rgba(0,0,0,0.45)", display: "flex", alignItems: "center", justifyContent: "center", zIndex: 40 }}>
      <div style={{ width: 470, background: C.surface, borderRadius: 14, padding: 28, display: "flex", flexDirection: "column", gap: 14 }}>
        <div style={{ fontSize: 18, color: C.text }}>
          {heading}{cut && "，已断开账号页面"}
        </div>

        <div style={{ background: C.bg, borderRadius: 8, padding: "12px 14px", fontSize: 12, color: C.sub, lineHeight: 1.8 }}>
          探测出口：<b style={{ color: splitTunnel ? C.warn : C.danger }}>{api.regionLabel(verdict)}</b>
          {verdict.ip && <span style={{ color: C.muted }}>（{verdict.ip}）</span>}
          <br />
          {cut
            ? "账号页面已经断开连接，不会再发出任何请求（勾了「网络异常时也保活、不断开」的账号除外）。"
            : "账号页面还连着，正在继续发请求。"}
        </div>

        <div style={{ fontSize: 13, color: C.sub, lineHeight: 1.8 }}>
          {splitTunnel
            ? "国内探针仍显示大陆出口，但外网探针可达。目标站点可能走境外节点，因此按网络异常处理。"
            : "阿里云日志等服务因政策原因不允许境外访问，持续访问可能触发账号风控。"}
          {cut && <><br />登录态已经存好了，关掉代理重新连上就能接着用，不用重新登录。</>}
        </div>

        {failed && (
          <div style={{ fontSize: 12, color: C.danger }}>网络仍不符合放行条件。请检查代理 / VPN 后重试。</div>
        )}

        <div style={{ display: "flex", gap: 10, justifyContent: "flex-end", alignItems: "center", flexWrap: "wrap" }}>
          <span onClick={onSettings} style={{ fontSize: 12, color: C.muted, cursor: "pointer", marginRight: "auto" }}>
            去设置
          </span>
          {cut ? (
            <Button
              kind="primary"
              disabled={busy}
              onClick={async () => {
                setBusy(true);
                setFailed(!(await onRecheck()));
                setBusy(false);
              }}
            >
              {busy ? "检测中…" : "已关闭代理，重新连接"}
            </Button>
          ) : (
            <>
              <span onClick={onIgnore} style={{ fontSize: 12, color: C.muted, cursor: "pointer" }}>
                继续用着
              </span>
              <Button kind="primary" onClick={onCut}>
                断开账号页面
              </Button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

/** 设置页里的「网络区域检测」区块 */
export function RegionSettings({ onChanged }: { onChanged?: (s: api.RegionState) => void }) {
  const [state, setState] = useState<api.RegionState | null>(null);
  const [busy, setBusy] = useState(false);
  const [savingInterval, setSavingInterval] = useState(false);
  const [err, setErr] = useState("");
  const [editing, setEditing] = useState<string | null>(null);
  const [msg, setMsg] = useState("");

  async function load(force: boolean) {
    setErr("");
    setBusy(true);
    try {
      let s = force ? await api.patrolRegion() : await api.checkRegion(false);
      if (!force && s.mode !== "off" && api.regionNeedsHandling(s.verdict)) {
        s = await api.patrolRegion();
      }
      setState(s);
      onChanged?.(s);
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  // 首次进设置页时拉一次（用缓存，不强制重测）。
  // 注意别写成 useState(() => ...)——那是 render 期执行，StrictMode 下会双发。
  useEffect(() => {
    void load(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function pick(mode: RegionMode) {
    setErr("");
    try {
      await api.setRegionMode(mode);
      await load(mode !== "off");
    } catch (e) {
      setErr(String(e));
    }
  }

  async function pickInterval(secs: number) {
    if (savingInterval || state?.watchSecs === secs) return;
    setErr("");
    setSavingInterval(true);
    try {
      await api.setRegionWatchSecs(secs);
      setState((s) => s ? { ...s, watchSecs: secs } : s);
    } catch (e) {
      setErr(String(e));
    } finally {
      setSavingInterval(false);
    }
  }

  const modeDescriptions: Record<RegionMode, string> = {
    block: "境外出口或检测到分流代理 / VPN 时阻止打开账号页；运行中会断开页面，网络恢复后自动接回。",
    warn: "境外出口或检测到分流代理 / VPN 时弹出提示，由你决定是否断开页面。",
    off: "不自动检测；仍可在顶栏手动检测。",
  };
  const option = (mode: RegionMode, title: string) => (
    <label
      key={mode}
      style={{ display: "flex", gap: 7, alignItems: "center", justifyContent: "center", cursor: "pointer", padding: "8px 10px", border: `1px solid ${state?.mode === mode ? C.brand : C.border}`, borderRadius: 7, background: state?.mode === mode ? C.brandSoft : C.surface, color: state?.mode === mode ? C.brand : C.sub, fontSize: 12, fontWeight: state?.mode === mode ? 600 : 400, flex: "1 1 140px", whiteSpace: "nowrap" }}
    >
      <input
        type="radio"
        checked={state?.mode === mode}
        onChange={() => pick(mode)}
        style={{ margin: 0, width: 14, flexShrink: 0 }}
      />
      {title}
    </label>
  );

  const v = state?.verdict;
  const t = v ? tone(v) : null;

  return (
    <Card style={{ display: "flex", flexDirection: "column", gap: 16 }}>
      <div style={{ fontSize: 15, color: C.text }}>网络区域检测</div>
      <div style={{ fontSize: 12, color: C.sub, lineHeight: 1.6 }}>
        阿里云日志等服务不允许境外访问。打开账号页前检测网络出口，并按设置的间隔复查。
        仅未检测到分流代理 / VPN 的中国大陆网络放行，港澳台按境外处理。
      </div>

      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        <div role="radiogroup" aria-label="网络区域检测模式" style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
          {option("block", "开启拦截（推荐）")}
          {option("warn", "仅提示")}
          {option("off", "关闭")}
        </div>
        <div style={{ fontSize: 12, color: C.muted, lineHeight: 1.6, minHeight: 19 }}>
          {state ? modeDescriptions[state.mode] : "正在读取检测设置…"}
        </div>
      </div>

      <div style={{ display: "flex", flexDirection: "column", gap: 8, fontSize: 12, color: C.sub }}>
        <div>自动复查间隔 <span style={{ color: C.muted }}>（仅开启检测时生效）</span></div>
        <div role="group" aria-label="自动复查间隔" style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
          {([
            [60, "1 分钟"],
            [300, "5 分钟"],
            [900, "15 分钟"],
            [3600, "1 小时"],
            [7200, "2 小时"],
            [86400, "1 天"],
          ] as const).map(([secs, label]) => {
            const selected = state?.watchSecs === secs;
            return (
              <button
                key={secs}
                type="button"
                aria-pressed={selected}
                disabled={!state || savingInterval}
                onClick={() => void pickInterval(secs)}
                style={{
                  padding: "6px 10px",
                  borderRadius: 6,
                  border: `1px solid ${selected ? C.brand : C.border}`,
                  background: selected ? C.brandSoft : C.surface,
                  color: selected ? C.brand : C.sub,
                  fontSize: 12,
                  cursor: !state || savingInterval ? "default" : "pointer",
                  opacity: !state || savingInterval ? 0.65 : 1,
                }}
              >
                {label}
              </button>
            );
          })}
        </div>
      </div>

      {state?.mode !== "off" && (
        <div style={{ background: C.bg, borderRadius: 8, padding: "12px 14px", display: "flex", flexDirection: "column", gap: 8, marginTop: 4 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
            <span style={{ fontSize: 13, color: busy ? C.muted : v?.origin.kind === "mainland" && v.tunnel ? C.warn : t?.color ?? C.muted }}>
              {busy ? "正在检测当前出口…" : <>{t?.flag} 探测出口：{v ? api.regionLabel(v) : "尚未检测"}</>}
              {!busy && v?.ip && <span style={{ color: C.muted, fontSize: 12 }}>（{v.ip}）</span>}
            </span>
            <Button onClick={() => load(true)} disabled={busy} style={{ padding: "4px 10px", fontSize: 12, marginLeft: "auto" }}>
              {busy ? "检测中…" : "重新检测"}
            </Button>
          </div>

          {!busy && v?.origin.kind === "mainland" && v.tunnel && (
            <div style={{ fontSize: 12, color: C.warn, lineHeight: 1.7 }}>
              ⚠️ 检测到分流代理 / VPN：外网探针可达，但国内探测域名仍走大陆出口。
              目标站点的实际出口取决于代理规则；已按当前模式将此状态作为网络异常处理。
            </div>
          )}
          {!busy && v?.origin.kind === "unknown" && (
            <div style={{ fontSize: 12, color: C.muted, lineHeight: 1.7 }}>
              探测失败（{v.origin.reason}），<b>不会拦截</b>——测不到通常是断网或服务挂了，不是你在境外。
            </div>
          )}
          {!busy && v?.proxy && (
            <div style={{ fontSize: 11, color: C.muted, lineHeight: 1.6 }}>
              检测到系统代理，结果代表探测请求的出口；若代理对某个站单独设了规则，实际路径可能不同。
            </div>
          )}
        </div>
      )}

      {state && state.mode !== "off" && (
        <div style={{ marginTop: 2 }}>
          <span
            onClick={() => setEditing(editing === null ? JSON.stringify(state.endpoints, null, 2) : null)}
            style={{ fontSize: 12, color: C.brand, cursor: "pointer" }}
          >
            {editing === null ? "探测端点设置" : "收起"}
          </span>
        </div>
      )}

      {editing !== null && (
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          <div style={{ fontSize: 12, color: C.muted, lineHeight: 1.7 }}>
            端点挂了或改了格式，改这里就行，不用重新发版。
            <code>*_path</code> 是点号路径，数字段表示数组下标（如 <code>data.location.0</code>）。
            <b>必须用 HTTPS</b>——明文响应可被伪造，这道闸就白设了。
          </div>
          <textarea
            value={editing}
            onChange={(e) => setEditing(e.target.value)}
            rows={10}
            style={{ ...inputStyle, fontFamily: "ui-monospace, monospace", fontSize: 11, resize: "vertical" }}
          />
          <div style={{ display: "flex", gap: 10 }}>
            <Button
              kind="primary"
              onClick={async () => {
                setErr("");
                setMsg("");
                try {
                  const n = await api.setRegionEndpoints(editing);
                  setMsg(`已保存 ${n} 个端点`);
                  setEditing(null);
                  await load(true);
                } catch (e) {
                  setErr(String(e));
                }
              }}
            >
              保存并重新检测
            </Button>
            <Button
              onClick={async () => {
                try {
                  setEditing(await api.resetRegionEndpoints());
                  setMsg("已恢复默认端点，记得保存");
                } catch (e) {
                  setErr(String(e));
                }
              }}
            >
              恢复默认
            </Button>
          </div>
        </div>
      )}

      <ErrorLine text={err} />
      {msg && <div style={{ fontSize: 12, color: C.brand }}>{msg}</div>}
    </Card>
  );
}
