import { useEffect, useMemo, useState } from "react";
import * as api from "../api";
import type { Account, SessionInfo } from "../api";
import { C, MOD } from "../ui";
import { FilterGroup, NavItem, Sidebar, SideNav, type Page } from "./Layout";

type Filter = { kind: "all" } | { kind: "owner"; value: string } | { kind: "platform"; value: string };

type Props = {
  accounts: Account[];
  sessions: SessionInfo[];
  onAdd: () => void;
  onEdit: (a: Account) => void;
  onDuplicate: (a: Account) => void;
  onDelete: (a: Account) => void;
  onLaunch: (a: Account) => void;
  onTotp: (a: Account) => void;
  /** 复制明文统一走 App 的主密码闸门，返回"复制成功 / 失败 / 用户取消" */
  onCopy: (label: string, text: api.CopySource) => Promise<"done" | "error" | "cancel">;
  onLock: () => void;
  onGo: (p: Page) => void;
};

function countBy(accounts: Account[], key: "owner_type" | "platform") {
  const m = new Map<string, number>();
  for (const a of accounts) m.set(a[key], (m.get(a[key]) ?? 0) + 1);
  return [...m.entries()];
}

export default function Main(p: Props) {
  const [filter, setFilter] = useState<Filter>({ kind: "all" });
  const [q, setQ] = useState("");
  const [blurred, setBlurred] = useState(false);
  const online = useMemo(() => new Set(p.sessions.map((s) => s.account_id)), [p.sessions]);

  const shown = useMemo(() => {
    const kw = q.trim().toLowerCase();
    return p.accounts.filter((a) => {
      if (filter.kind === "owner" && a.owner_type !== filter.value) return false;
      if (filter.kind === "platform" && a.platform !== filter.value) return false;
      if (!kw) return true;
      return [a.related_app, a.remark, a.username, a.platform, a.owner_type].some((s) => s.toLowerCase().includes(kw));
    });
  }, [p.accounts, filter, q]);

  return (
    <div style={{ flexGrow: 1, minWidth: 0, display: "flex", background: C.white }}>
      <Sidebar footer={<SideNav current="main" sessionCount={p.sessions.length} onGo={p.onGo} onLock={p.onLock} />}>
        <div style={{ display: "flex", flexDirection: "column", gap: 18 }}>
          <NavItem
            label="全部账号"
            count={p.accounts.length}
            active={filter.kind === "all"}
            onClick={() => setFilter({ kind: "all" })}
          />

          <FilterGroup
            title="归属"
            items={countBy(p.accounts, "owner_type")}
            activeValue={filter.kind === "owner" ? filter.value : null}
            blurValues={blurred}
            onPick={(v) => setFilter(filter.kind === "owner" && filter.value === v ? { kind: "all" } : { kind: "owner", value: v })}
          />

          <FilterGroup
            title="平台"
            items={countBy(p.accounts, "platform")}
            activeValue={filter.kind === "platform" ? filter.value : null}
            onPick={(v) => setFilter(filter.kind === "platform" && filter.value === v ? { kind: "all" } : { kind: "platform", value: v })}
          />
        </div>
      </Sidebar>

      <div style={{ flexGrow: 1, display: "flex", flexDirection: "column", minWidth: 0 }}>
        <div
          data-tauri-drag-region
          style={{
            height: 72,
            flexShrink: 0,
            boxSizing: "border-box",
            padding: "0 32px",
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            borderBottom: `1px solid ${C.border}`,
          }}
        >
          <div style={{ display: "flex", alignItems: "center", gap: 14 }}>
            <input
              placeholder={blurred ? "搜索内容已隐藏" : `搜索应用名 / 备注...（${MOD}K 快捷切换）`}
              value={blurred ? "" : q}
              onChange={(e) => setQ(e.target.value)}
              aria-label="搜索账号"
              disabled={blurred}
              style={{
                width: 300,
                boxSizing: "border-box",
                padding: "9px 12px",
                borderRadius: 7,
                border: `1px solid ${C.border}`,
                background: C.bg,
                fontSize: 13,
              }}
            />
            <div
              onClick={() => p.onGo("sessions")}
              style={{
                padding: "9px 14px",
                borderRadius: 7,
                background: p.sessions.length ? C.brandSoft : C.bg,
                color: p.sessions.length ? C.brand : C.muted,
                fontSize: 12,
                fontWeight: 500,
                cursor: "pointer",
              }}
            >
              {p.sessions.length} 个账号在线
            </div>
          </div>
          <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
            <button
              type="button"
              aria-pressed={blurred}
              onClick={() => setBlurred((value) => !value)}
              title={blurred ? "退出截图模糊模式" : "模糊账号列表，用于截图"}
              style={{ display: "flex", alignItems: "center", gap: 6, padding: "8px 12px", borderRadius: 7, border: `1px solid ${blurred ? C.brand : C.border}`, background: blurred ? C.brandSoft : C.white, color: blurred ? C.brand : C.sub, fontSize: 12, cursor: "pointer", whiteSpace: "nowrap" }}
            >
              <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
                <path d="M2 12s3.7-6 10-6 10 6 10 6-3.7 6-10 6S2 12 2 12Z" />
                <circle cx="12" cy="12" r="3" />
                {blurred && <path d="M3 3l18 18" />}
              </svg>
              {blurred ? "显示列表" : "模糊列表"}
            </button>
            <button
              onClick={p.onAdd}
              style={{ padding: "9px 16px", borderRadius: 7, border: "none", background: C.brand, color: C.white, fontSize: 13, fontWeight: 500, cursor: "pointer", whiteSpace: "nowrap" }}
            >
              + 新增账号
            </button>
          </div>
        </div>

        {p.accounts.length === 0 ? (
          <EmptyState onAdd={p.onAdd} />
        ) : shown.length === 0 ? (
          <SearchEmpty keyword={q} masked={blurred} onAdd={p.onAdd} />
        ) : (
          <div
            style={{
              flexGrow: 1,
              minHeight: 0,
              overflowY: "auto",
              padding: "24px 32px",
            }}
          >
            <div style={{ display: "grid", gridTemplateColumns: "repeat(3, minmax(0, 1fr))", gap: 16, alignContent: "start" }}>
              {shown.map((a) => (
                <AccountCard key={a.id} a={a} online={online.has(a.id)} blurred={blurred} p={p} />
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function AccountCard({ a, online, blurred, p }: { a: Account; online: boolean; blurred: boolean; p: Props }) {
  const [copyStatus, setCopyStatus] = useState<"idle" | "done" | "error">("idle");
  useEffect(() => {
    if (copyStatus === "idle") return;
    const timer = window.setTimeout(() => setCopyStatus("idle"), 2500);
    return () => window.clearTimeout(timer);
  }, [copyStatus]);

  /** 单个字段的复制。空值的按钮本身是禁用的，所以这里只管结果三态 */
  async function copyField(label: string, value: api.CopySource) {
    const r = await p.onCopy(label, value);
    setCopyStatus(r === "done" ? "done" : r === "error" ? "error" : "idle");
  }

  async function copyAccount() {
    // 列表里的密码是打过码的，验完主密码再去后端取完整的一条
    const r = await p.onCopy("这个账号的全部信息", async () => {
      const f = await api.getAccount(a.id);
      const fields = [
        ["平台", f.platform],
        ["归属", f.owner_type],
        ["关联应用 / 项目", f.related_app],
        ["账号角色", f.account_role],
        ["登录直达 URL", f.login_url],
        ["登录名 / 邮箱", f.username],
        ["密码", f.credential],
        ["TOTP 密钥", f.totp_secret],
        ["验证码联系人备注", f.backup_contact],
        ["备注", f.remark],
        ...f.extra_fields.map((field): [string, string] => [field.label, field.value]),
      ];
      return fields.filter(([, value]) => value.trim()).map(([label, value]) => `${label}：${value}`).join("\n");
    });
    setCopyStatus(r === "done" ? "done" : r === "error" ? "error" : "idle");
  }

  const iconButton: React.CSSProperties = {
    width: 28,
    height: 28,
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    borderRadius: 6,
    border: `1px solid ${C.border}`,
    background: C.white,
    color: C.sub,
    cursor: "pointer",
  };
  const small = (color: string): React.CSSProperties => ({
    padding: "7px 12px",
    borderRadius: 6,
    border: `1px solid ${C.border}`,
    background: C.white,
    color,
    fontSize: 12,
  });
  const maskedText: React.CSSProperties = { filter: blurred ? "blur(8px)" : "none", userSelect: blurred ? "none" : "auto" };
  return (
    <div
      style={{
        border: `1px solid ${online ? "rgba(0,168,112,0.4)" : C.border}`,
        borderRadius: 10,
        padding: 16,
        display: "flex",
        flexDirection: "column",
        gap: 10,
        background: C.white,
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <span style={{ fontSize: 11, padding: "3px 8px", borderRadius: 999, background: C.brandSoft, color: C.brand }}>{a.platform}</span>
        <span style={{ display: "inline-block", fontSize: 11, color: C.muted, ...maskedText }}>{a.owner_type}</span>
        <div style={{ marginLeft: "auto", display: "flex", alignItems: "center", gap: 6, flexShrink: 0 }}>
          {online && (
            <span style={{ fontSize: 11, color: "#00A870", display: "flex", alignItems: "center", gap: 4, marginRight: 2 }}>
              <span style={{ width: 6, height: 6, borderRadius: 999, background: "#00A870" }} />
              在线
            </span>
          )}
          <button type="button" onClick={() => p.onDuplicate(a)} title="复制新建账号" aria-label="复制新建账号" style={iconButton}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
              <path d="M8 4h9a2 2 0 0 1 2 2v10" />
              <rect x="4" y="8" width="11" height="12" rx="2" />
              <path d="M9.5 11.5v5M7 14h5" />
            </svg>
          </button>
          <button type="button" onClick={() => void copyAccount()} title="复制账号信息到剪贴板（含密码、TOTP 密钥和附加字段）" aria-label="复制账号信息到剪贴板" style={{ ...iconButton, color: copyStatus === "done" ? "#00A870" : copyStatus === "error" ? C.danger : C.sub }}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
              <rect x="5" y="5" width="14" height="16" rx="2" />
              <path d="M9 5.5V3h6v2.5M9 10h6M9 14h6" />
            </svg>
          </button>
        </div>
      </div>
      {copyStatus !== "idle" && <div role="status" style={{ fontSize: 11, color: copyStatus === "done" ? "#00A870" : C.danger }}>{copyStatus === "done" ? "已复制到剪贴板" : "复制失败，请检查剪贴板权限"}</div>}
      <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
        <div style={{ fontSize: 14, color: C.text, fontWeight: 500, ...maskedText }}>{a.related_app || "（未填写应用）"}</div>
        <div style={{ fontSize: 12, color: C.sub, ...maskedText }}>
          {a.username} · {a.account_role}
        </div>
      </div>
      {/* 单独复制：账号库里存的明文只有这三样是平时要往别处粘的 */}
      <div style={{ display: "flex", alignItems: "center", gap: 6, flexWrap: "wrap" }}>
        <span style={{ fontSize: 11, color: C.muted }}>复制</span>
        {([
          ["账号", a.username, a.username],
          // 列表里的密码打过码，只能拿来判断"有没有"；真值验完主密码再取
          ["密码", a.credential, () => api.getAccount(a.id).then((f) => f.credential)],
          ["访问地址", a.login_url, a.login_url],
        ] as const).map(([label, value, source]) => (
          <button
            key={label}
            type="button"
            disabled={!value.trim()}
            onClick={() => void copyField(label, source)}
            title={value.trim() ? `验证主密码后复制${label}` : `还没填${label}`}
            style={{
              padding: "4px 9px",
              borderRadius: 6,
              border: `1px solid ${C.border}`,
              background: C.white,
              color: value.trim() ? C.sub : C.muted,
              fontSize: 11,
              opacity: value.trim() ? 1 : 0.5,
              cursor: value.trim() ? "pointer" : "default",
            }}
          >
            {label}
          </button>
        ))}
      </div>
      <div style={{ fontSize: 12, color: C.muted, minHeight: 16, ...maskedText }}>{a.remark}</div>
      <div style={{ display: "flex", gap: 8, marginTop: 2, flexWrap: "wrap" }}>
        <button
          onClick={() => (a.login_url.trim() ? p.onLaunch(a) : p.onEdit(a))}
          title={blurred ? "打开账号页面" : a.login_url.trim() || "还没填登录直达 URL，点这里去补"}
          style={{
            padding: "7px 12px",
            borderRadius: 6,
            border: "none",
            background: a.login_url.trim() ? C.brand : C.borderStrong,
            color: C.white,
            fontSize: 12,
            fontWeight: 500,
          }}
        >
          {online ? "切过去" : a.login_url.trim() ? "一键登录" : "补登录地址"}
        </button>
        {a.totp_secret && (
          <button onClick={() => p.onTotp(a)} style={small(C.text)}>
            验证码
          </button>
        )}
        {online && (
          <button onClick={() => p.onGo("sessions")} style={small(C.sub)} title="到会话页手动填充 / 指认输入框">
            手动填充
          </button>
        )}
        <button onClick={() => p.onEdit(a)} style={small(C.sub)}>
          编辑
        </button>
        <button onClick={() => p.onDelete(a)} style={small(C.danger)}>
          删除
        </button>
      </div>
    </div>
  );
}

function EmptyState({ onAdd }: { onAdd: () => void }) {
  return (
    <div style={{ flexGrow: 1, display: "flex", alignItems: "center", justifyContent: "center" }}>
      <div style={{ width: 380, display: "flex", flexDirection: "column", alignItems: "center", gap: 18, textAlign: "center" }}>
        <div
          style={{
            width: 64,
            height: 64,
            borderRadius: 16,
            border: `1px solid ${C.border}`,
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            fontSize: 24,
            color: C.brand,
          }}
        >
          +
        </div>
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <div style={{ fontSize: 20, color: C.text }}>账号库还是空的</div>
          <div style={{ fontSize: 13, color: C.sub, lineHeight: 1.6 }}>
            把阿里云、腾讯云、即构这些散落的账号登记进来
            <br />
            每条记录会自动获得独立的隔离登录环境
          </div>
        </div>
        <button onClick={onAdd} style={{ padding: "10px 20px", borderRadius: 8, border: "none", background: C.brand, color: C.white, fontSize: 14, fontWeight: 500 }}>
          + 添加第一个账号
        </button>
      </div>
    </div>
  );
}

function SearchEmpty({ keyword, masked, onAdd }: { keyword: string; masked: boolean; onAdd: () => void }) {
  return (
    <div style={{ flexGrow: 1, display: "flex", alignItems: "center", justifyContent: "center" }}>
      <div style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: 14, textAlign: "center" }}>
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <div style={{ fontSize: 14, color: C.text }}>{masked ? "没有匹配的账号" : `没有匹配“${keyword}”的账号`}</div>
          <div style={{ fontSize: 12, color: C.muted }}>检查一下平台/应用名有没有拼错，或者这本来就是个新账号</div>
        </div>
        <button onClick={onAdd} style={{ padding: "8px 16px", borderRadius: 7, border: `1px solid ${C.borderStrong}`, background: C.white, color: C.text, fontSize: 13 }}>
          {masked ? "直接新增账号" : `直接新增“${keyword}”账号`}
        </button>
      </div>
    </div>
  );
}
