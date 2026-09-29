import { useEffect, useState } from "react";
import * as api from "../api";
import type { Account, PlatformConfig } from "../api";
import { Button, C, ErrorLine, inputStyle } from "../ui";
import { NavItem, SideGroup, SubPage, type SubPageShell } from "./Layout";

const COLS = "1.1fr 1.4fr 1.4fr 0.9fr 0.8fr 0.6fr";

/** 侧栏筛选的顺序：先放要动手的 */
const STATUS_ORDER = ["待配置", "未验证", "已验证", "走扫码，跳过"];

function status(c: PlatformConfig) {
  if (c.trigger_event === "skip") return { text: "走扫码，跳过", color: C.muted };
  if (!c.username_selector || !c.password_selector) return { text: "待配置", color: C.warn };
  return c.verified ? { text: "已验证", color: C.success } : { text: "未验证", color: C.muted };
}

/** PlatformConfig.dc.html —— 选择器配置表，可直接编辑，也可由"手动指认"写回 */
export default function PlatformConfigScreen({
  shell,
  accounts,
  onPick,
}: {
  shell: SubPageShell;
  accounts: Account[];
  /** 打开该平台的一个账号并进入指认模式，由 App 统一处理（要过区域检测那道闸） */
  onPick: (platform: string) => void;
}) {
  const [rows, setRows] = useState<PlatformConfig[]>([]);
  const [editing, setEditing] = useState<PlatformConfig | null>(null);
  const [err, setErr] = useState("");
  /** 侧栏选的状态，null = 全部 */
  const [only, setOnly] = useState<string | null>(null);
  const shown = only ? rows.filter((c) => status(c).text === only) : rows;
  const count = (text: string) => rows.filter((c) => status(c).text === text).length;

  const load = () => api.listPlatformConfigs().then(setRows).catch((e) => setErr(String(e)));
  useEffect(() => {
    load();
  }, []);

  /** 只删没人用的配置（「其他 · 旧主机」这类），内置的、还有账号在用的后端会拒并说原因 */
  async function remove() {
    if (!editing) return;
    setErr("");
    try {
      await api.deletePlatformConfig(editing.platform);
      setEditing(null);
      await load();
    } catch (e) {
      setErr(String(e));
    }
  }

  async function save() {
    if (!editing) return;
    try {
      await api.savePlatformConfig(editing);
      setEditing(null);
      await load();
    } catch (e) {
      setErr(String(e));
    }
  }

  const cell: React.CSSProperties = { fontSize: 12, color: C.sub, fontFamily: "ui-monospace, monospace", overflow: "hidden", textOverflow: "ellipsis" };

  return (
    <SubPage
      current="platforms"
      {...shell}
      side={
        <SideGroup title="状态">
          <NavItem label="全部平台" count={rows.length} active={only === null} onClick={() => setOnly(null)} />
          {/* 数量为 0 的不列；正选着的那个就算刚被配完变成 0 也留着，不然高亮没了、用户不知道自己在看什么 */}
          {STATUS_ORDER.filter((t) => count(t) > 0 || only === t).map((t) => (
            <NavItem key={t} label={t} count={count(t)} active={only === t} onClick={() => setOnly(only === t ? null : t)} />
          ))}
        </SideGroup>
      }
    >
    <div style={{ display: "flex", flexDirection: "column", gap: 20, maxWidth: 940 }}>
      <div>
        <h1 style={{ margin: 0, fontWeight: 500, fontSize: 24, color: C.text }}>平台适配配置</h1>
        <div style={{ fontSize: 13, color: C.sub, marginTop: 6 }}>
          每个平台登录页要找的输入框，存成独立配置，平台改版了改这里就行，不用重新编译发版
        </div>
      </div>

      <div style={{ border: `1px solid ${C.border}`, borderRadius: 10, overflow: "hidden" }}>
        <div style={{ display: "grid", gridTemplateColumns: COLS, gap: 12, padding: "12px 16px", background: C.bg, fontSize: 12, color: C.muted }}>
          <div>平台</div>
          <div>账号输入框选择器</div>
          <div>密码输入框选择器</div>
          <div>触发事件</div>
          <div>状态</div>
          <div />
        </div>
        {only && shown.length === 0 && (
          <div style={{ padding: "16px", borderTop: `1px solid ${C.border}`, fontSize: 13, color: C.muted }}>
            已经没有「{only}」的平台了
          </div>
        )}
        {shown.map((c) => {
          const s = status(c);
          return (
            <div
              key={c.platform}
              style={{ display: "grid", gridTemplateColumns: COLS, gap: 12, padding: "12px 16px", borderTop: `1px solid ${C.border}`, alignItems: "center" }}
            >
              <div style={{ fontSize: 13, color: C.text }}>{c.platform}</div>
              <div style={cell}>{c.username_selector || "未配置"}</div>
              <div style={cell}>{c.password_selector || "未配置"}</div>
              <div style={cell}>{c.trigger_event || "—"}</div>
              <div style={{ fontSize: 12, color: s.color }}>{s.text}</div>
              <div style={{ textAlign: "right" }}>
                <Button onClick={() => { setErr(""); setEditing(c); }} style={{ padding: "5px 10px", fontSize: 12 }}>
                  编辑
                </Button>
              </div>
            </div>
          );
        })}
      </div>

      <div style={{ fontSize: 12, color: C.muted, lineHeight: 1.6 }}>
        “未配置”的平台，到「会话」页打开该账号，点左边的「指认输入框」，再到右边页面依次点一次账号框和密码框，选完会自动写回这张表，下次直接生效。
        <br />
        触发事件填 <code>input</code> 或 <code>input+change</code>；走扫码不需要填充的平台填 <code>skip</code>。
      </div>

      {/* 弹框开着时错误显示在框里，不然压在遮罩后面看不清 */}
      {!editing && <ErrorLine text={err} />}

      {editing && (
        <div
          onClick={() => setEditing(null)}
          style={{ position: "fixed", inset: 0, background: "rgba(0,0,0,0.35)", display: "flex", alignItems: "center", justifyContent: "center" }}
        >
          <div
            onClick={(e) => e.stopPropagation()}
            style={{ width: 460, background: C.surface, borderRadius: 14, padding: 28, display: "flex", flexDirection: "column", gap: 14 }}
          >
            <div style={{ fontSize: 18, color: C.text }}>{editing.platform}</div>

            {/* 手填选择器只是兜底。能点得到页面的话，指认一次比自己写 CSS 选择器靠谱得多 */}
            {(() => {
              const has = api.accountsForConfig(accounts, editing.platform).length > 0;
              return (
                <div style={{ background: C.bg, borderRadius: 8, padding: "12px 14px", display: "flex", flexDirection: "column", gap: 8 }}>
                  <Button kind="primary" disabled={!has} onClick={() => onPick(editing.platform)} style={{ padding: "8px 14px", fontSize: 13 }}>
                    在登录页上指认输入框
                  </Button>
                  <div style={{ fontSize: 12, color: C.muted, lineHeight: 1.7 }}>
                    {has
                      ? `会打开「${editing.platform}」的一个账号，页面出来后自动进入指认模式：先点账号框（标 1），再点密码框（标 2），选完自动写回这张表。`
                      : `账号库里还没有「${editing.platform}」的账号，没有登录页可点。先去账号列表加一个，或者直接在下面手填选择器。`}
                  </div>
                </div>
              );
            })()}

            {(
              [
                ["账号输入框选择器", "username_selector", "#username"],
                ["密码输入框选择器", "password_selector", "#password"],
                ["触发事件", "trigger_event", "input+change"],
              ] as const
            ).map(([label, key, ph]) => (
              <div key={key} style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                <label style={{ fontSize: 12, color: C.sub }}>{label}</label>
                <input
                  style={{ ...inputStyle, fontFamily: "ui-monospace, monospace" }}
                  placeholder={ph}
                  value={editing[key]}
                  onChange={(e) => setEditing({ ...editing, [key]: e.target.value })}
                />
              </div>
            ))}
            <label style={{ fontSize: 13, color: C.sub, display: "flex", alignItems: "center", gap: 8 }}>
              <input type="checkbox" checked={editing.verified} onChange={(e) => setEditing({ ...editing, verified: e.target.checked })} />
              标记为已验证
            </label>
            {editing.updated_at && <div style={{ fontSize: 12, color: C.muted }}>上次更新：{editing.updated_at}</div>}
            <ErrorLine text={err} />
            <div style={{ display: "flex", gap: 10, justifyContent: "flex-end" }}>
              <Button kind="danger" onClick={remove} style={{ marginRight: "auto" }}>
                删除这条配置
              </Button>
              <Button onClick={() => setEditing(null)}>取消</Button>
              <Button kind="primary" onClick={save}>
                保存
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
    </SubPage>
  );
}
