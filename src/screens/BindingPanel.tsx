import { useEffect, useState } from "react";
import * as api from "../api";
import { C } from "../ui";

const messages: Record<string, string> = {
  pending: "等待中", filled: "已填写", already_matches: "值已一致", nonempty_skipped: "已有内容，已跳过",
  target_not_found: "没有找到输入框", ambiguous_target: "匹配到多个框，请重新指认", unsupported_target: "输入框不可编辑或不支持",
  value_rejected: "网页未接受填写值", missing_source: "来源字段已删除", empty_source: "来源没有可用值",
  page_mismatch: "页面不匹配", frame_mismatch: "内嵌页面不匹配", stale_document: "页面已变化，请重试",
  cancelled: "已取消", timeout: "等待超时", vault_locked: "账号库已锁定", session_cut: "会话已断开",
  running: "正在填写", completed: "填写完成", partial: "部分完成", failed: "未完成",
};
const keyOf = (ref: api.BindingSource) => ref.kind === "account" ? ref.field : ref.kind === "extra" ? `extra:${ref.fieldId}` : "totp";
const refOf = (key: string): api.BindingSource => key.startsWith("extra:") ? { kind: "extra", fieldId: key.slice(6) }
  : key === "totp" ? { kind: "totp" } : { kind: "account", field: key as "username" | "password" };
const matches = (a: api.BindingPage, b: api.BindingPage | null) => !!b && a.origin === b.origin && a.pathname === b.pathname
  && a.hashRoute === b.hashRoute && Object.keys(a.queryEquals).length === Object.keys(b.queryEquals).length && Object.entries(a.queryEquals).every(([k, v]) => b.queryEquals[k] === v);
const pageName = (p: api.BindingPage) => p.origin + p.pathname + (Object.keys(p.queryEquals).length ? `?${new URLSearchParams(p.queryEquals)}` : "") + (p.hashRoute ? `#${p.hashRoute}` : "");

export default function BindingPanel({ account, tabLabel, ready }: { account: api.Account; tabLabel: string; ready: boolean }) {
  const [state, setState] = useState<api.BindingState | null>(null);
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);
  const [editing, setEditing] = useState<api.FieldBinding | null>(null);
  const [source, setSource] = useState("username");
  const [name, setName] = useState("账号");
  const [auto, setAuto] = useState(true);
  const [overwrite, setOverwrite] = useState(false);
  const sources = [ { key: "username", label: "账号" }, { key: "password", label: "密码" }, { key: "totp", label: "实时验证码（TOTP）" },
    ...account.extra_fields.map((f, i) => ({ key: `extra:${f.id}`, label: `${f.label}（附加字段 ${i + 1}）` })) ];
  const sourceName = (ref: api.BindingSource) => sources.find(s => s.key === keyOf(ref))?.label || "来源字段已删除";
  const refresh = async () => setState(await api.bindingState(account.id, tabLabel));
  useEffect(() => {
    let alive = true;
    const fetch = () => { api.bindingState(account.id, tabLabel).then(s => { if (alive) setState(s); }).catch(() => { if (alive) setState(null); }); };
    fetch();
    const un = api.onBindingsChanged(fetch);
    const timer = setInterval(fetch, 2000);
    return () => { alive = false; clearInterval(timer); void un.then(f => f()); };
  }, [account.id, tabLabel]);
  const action = async (fn: () => Promise<unknown>) => {
    setBusy(true); setErr("");
    try { await fn(); await refresh(); } catch (e) { setErr(String(e)); } finally { setBusy(false); }
  };
  const button = (label: string, fn: () => Promise<unknown>, disabled = false) => (
    <button type="button" disabled={busy || disabled} onClick={() => void action(fn)} style={{ border: `1px solid ${C.border}`, borderRadius: 5, background: C.surface, color: C.text, fontSize: 11, padding: "5px 7px", cursor: "pointer", opacity: busy || disabled ? 0.5 : 1 }}>{label}</button>
  );
  const running = state?.run?.status === "running";
  const choose = (value: string) => {
    setSource(value); setName(sources.find(s => s.key === value)?.label || "字段");
    setAuto(value === "username" || value === "password");
  };
  const save = async (fill: boolean) => {
    const id = await api.saveFieldBinding(account.id, tabLabel, {
      id: editing?.id, pickId: state?.draft?.pickId, label: name, enabled: editing?.enabled ?? true, automatic: auto, valueRef: refOf(source),
    });
    setEditing(null);
    if (fill) await api.runFieldBindings(account.id, tabLabel, id, overwrite);
  };
  return <details style={{ fontSize: 11, color: C.sub, border: `1px solid ${C.border}`, borderRadius: 6, padding: "7px 6px", lineHeight: 1.6 }}>
    <summary style={{ cursor: "pointer", color: C.text }}>输入框绑定{state?.bindings.length ? ` · ${state.bindings.length}` : ""}</summary>
    <div style={{ display: "flex", flexDirection: "column", gap: 7, paddingTop: 7 }}>
      <span>选择网页输入框，再关联左侧字段。绑定仅用于此账号和指定页面；暂不支持跨站内嵌页面。</span>
      {state && <label><input type="checkbox" checked={state.enabled} disabled={busy} onChange={e => void action(() => api.setBindingMode(account.id, e.target.checked))} /> 使用字段绑定模式</label>}
      {!state?.enabled && <span>当前使用旧平台适配；保存首条绑定后自动切换。</span>}
      {state?.enabled && <span>旧平台自动填写已停用；取消勾选后刷新可切回。</span>}
      {state?.page && <span title={pageName(state.page)} style={{ overflowWrap: "anywhere", color: C.muted }}>当前：{pageName(state.page)}</span>}
      {button("＋ 选择输入框", async () => { setEditing(null); choose("username"); await api.beginBindingPick(account.id, tabLabel); }, !ready || running || state?.picking)}
      {state?.picking && <span style={{ color: C.brand }}>到右侧点击一个文本输入框，60 秒内有效。</span>}
      {(state?.draft || editing) && <div style={{ display: "flex", flexDirection: "column", gap: 6, borderLeft: `2px solid ${C.brand}`, paddingLeft: 7 }}>
        <strong>{state?.draft ? "已选中输入框" : "编辑绑定"}</strong>
        <span style={{ overflowWrap: "anywhere", color: C.muted }}>{(state?.draft?.target || editing?.target)?.locator.value}</span>
        <label>值来源<select aria-label="绑定值来源" value={source} onChange={e => choose(e.target.value)} style={{ width: "100%", fontSize: 11, padding: 4 }}>
          {!sources.some(s => s.key === source) && <option value={source}>来源字段已删除，请重新选择</option>}
          {sources.map(s => <option key={s.key} value={s.key}>{s.label}</option>)}
        </select></label>
        <label>名称<input aria-label="绑定名称" value={name} maxLength={100} onChange={e => setName(e.target.value)} style={{ width: "100%", boxSizing: "border-box", fontSize: 11, padding: 4 }} /></label>
        <label><input type="checkbox" checked={auto} onChange={e => setAuto(e.target.checked)} /> 打开匹配页面时自动填写</label>
        <div style={{ display: "flex", flexWrap: "wrap", gap: 5 }}>{button("保存", () => save(false), !name.trim() || !ready || !sources.some(s => s.key === source))}{button("保存并试填", () => save(true), !name.trim() || !ready || !sources.some(s => s.key === source))}</div>
      </div>}
      {(state?.picking || state?.draft || editing || running) && button(running ? "取消填写" : "取消编辑 / 指认", async () => { setEditing(null); await api.cancelBindingWork(account.id, tabLabel); })}
      <label title="仅用于本次主动执行；自动填写始终保留已有内容"><input type="checkbox" checked={overwrite} onChange={e => setOverwrite(e.target.checked)} /> 主动填写时覆盖已有内容</label>
      {button("按绑定填写当前页", () => api.runFieldBindings(account.id, tabLabel, undefined, overwrite), !ready || !state?.enabled || running || !state.bindings.some(b => b.enabled && matches(b.target.page, state.page)))}
      {state?.bindings.map(b => <div key={b.id} style={{ paddingTop: 6, borderTop: `1px solid ${C.border}`, display: "flex", flexDirection: "column", gap: 4 }}>
        <strong style={{ color: C.text }}>{b.label}</strong><span>{sourceName(b.valueRef)} · {b.automatic ? "自动" : "手动"}{matches(b.target.page, state.page) ? " · 当前页" : ""}</span>
        <span title={pageName(b.target.page)} style={{ overflowWrap: "anywhere", color: C.muted }}>{pageName(b.target.page)}</span>
        <label><input type="checkbox" checked={b.enabled} disabled={busy || !ready} onChange={e => void action(() => api.saveFieldBinding(account.id, tabLabel, { id: b.id, label: b.label, enabled: e.target.checked, automatic: b.automatic, valueRef: b.valueRef }))} /> 启用</label>
        <div style={{ display: "flex", flexWrap: "wrap", gap: 5 }}>
          {button("填写", () => api.runFieldBindings(account.id, tabLabel, b.id, overwrite), !ready || !state.enabled || !b.enabled || running || !matches(b.target.page, state.page))}
          {button("编辑", async () => { await api.cancelBindingWork(account.id, tabLabel); setEditing(b); setSource(keyOf(b.valueRef)); setName(b.label); setAuto(b.automatic); }, !ready)}
          {button("重新指认", async () => { setEditing(b); setSource(keyOf(b.valueRef)); setName(b.label); setAuto(b.automatic); await api.beginBindingPick(account.id, tabLabel); }, !ready)}
          {button("删除", async () => { await api.deleteFieldBinding(account.id, b.id); if (editing?.id === b.id) setEditing(null); })}
        </div>
      </div>)}
      {state?.run && <div role="status" style={{ borderTop: `1px solid ${C.border}`, paddingTop: 6 }}>
        <strong>{messages[state.run.status] || state.run.status}</strong>
        {state.run.results.map(r => <div key={r.bindingId}>{r.label}：{messages[r.code] || r.code}</div>)}
      </div>}
      {err && <div role="alert" style={{ color: C.danger, overflowWrap: "anywhere" }}>{err}</div>}
    </div>
  </details>;
}
