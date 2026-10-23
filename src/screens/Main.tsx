import { useEffect, useMemo, useRef, useState } from "react";
import * as api from "../api";
import type { Account, SessionInfo } from "../api";
import { Button, C, IconButton, MOD, R } from "../ui";
import { IconClose, IconEye, IconEyeOff, IconMore, IconPlus } from "../icons";
import { FilterGroup, NavItem, Sidebar, SideNav, TopbarSlot, type Page } from "./Layout";

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
    <div style={{ flexGrow: 1, minWidth: 0, display: "flex", background: C.surface }}>
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

      {/* 上沿一条细线：卡片往上滚时有个边界，不然像是凭空被切掉 */}
      <div style={{ flexGrow: 1, display: "flex", flexDirection: "column", minWidth: 0, borderTop: `1px solid ${C.border}` }}>
        {/* 搜索和按钮放进顶栏那一行，不在顶栏下面再起一行（见 Layout 的 TopbarSlot）。
            顶栏整条是窗口拖动区，能点的都得是 button / input，拖动脚本才会让开 */}
        <TopbarSlot>
          <div className="sb-topbar-tools" style={{ flexGrow: 1, minWidth: 0, display: "flex", alignItems: "center", gap: 8, padding: "0 16px 0 32px" }}>
            <div className="sb-search-wrap" style={{ flex: "0 1 300px", minWidth: 140 }}>
              <input
                placeholder={blurred ? "搜索内容已隐藏" : `搜索应用名 / 备注...（${MOD}K 快捷切换）`}
                value={blurred ? "" : q}
                onChange={(e) => setQ(e.target.value)}
                aria-label="搜索账号"
                disabled={blurred}
                style={{
                  width: "100%",
                  height: 30,
                  boxSizing: "border-box",
                  padding: q && !blurred ? "0 30px 0 12px" : "0 12px",
                  borderRadius: "var(--sb-radius-sm)",
                  border: `1px solid ${C.border}`,
                  background: C.bg,
                  color: C.text,
                  fontSize: 12,
                }}
              />
              {q && !blurred && (
                <button type="button" className="sb-search-clear" aria-label="清除搜索" title="清除搜索" onClick={() => setQ("")}>
                  <IconClose size={12} />
                </button>
              )}
            </div>
            <button
              type="button"
              onClick={() => p.onGo("sessions")}
              title="到会话页"
              style={{
                height: 30,
                padding: "0 12px",
                border: 0,
                borderRadius: "var(--sb-radius-sm)",
                background: p.sessions.length ? C.brandSoft : C.bg,
                color: p.sessions.length ? C.brand : C.muted,
                fontSize: 12,
                fontWeight: 500,
                whiteSpace: "nowrap",
                flexShrink: 0,
              }}
            >
              {p.sessions.length} 个账号在线
            </button>
            <div style={{ flexGrow: 1 }} />
            <button
              type="button"
              aria-pressed={blurred}
              aria-label={blurred ? "退出截图模糊模式" : "模糊账号列表，用于截图"}
              onClick={() => setBlurred((value) => !value)}
              title={blurred ? "退出截图模糊模式" : "模糊账号列表，用于截图"}
              style={{ display: "flex", alignItems: "center", gap: 6, height: 30, padding: "0 12px", borderRadius: "var(--sb-radius-sm)", border: `1px solid ${blurred ? C.brand : C.border}`, background: blurred ? C.brandSoft : C.surface, color: blurred ? C.brand : C.sub, fontSize: 12, whiteSpace: "nowrap", flexShrink: 0 }}
            >
              {blurred ? <IconEyeOff size={14} /> : <IconEye size={14} />}
              <span className="sb-tool-label">{blurred ? "显示列表" : "模糊列表"}</span>
            </button>
            <Button kind="primary" size="sm" icon={<IconPlus size={14} />} onClick={p.onAdd} style={{ flexShrink: 0 }}>
              新增账号
            </Button>
          </div>
        </TopbarSlot>

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
            <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(280px, 1fr))", gap: 16, alignContent: "start" }}>
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
  const [menu, setMenu] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  // 点菜单外面就收起
  useEffect(() => {
    if (!menu) return;
    const close = (e: MouseEvent) => {
      if (!menuRef.current?.contains(e.target as Node)) setMenu(false);
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [menu]);
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

  const maskedText: React.CSSProperties = { filter: blurred ? "blur(8px)" : "none", userSelect: blurred ? "none" : "auto" };
  const copyLabel = copyStatus === "done" ? "已复制" : copyStatus === "error" ? "复制失败" : "复制";
  return (
    <div
      style={{
        border: `1px solid ${online ? C.successBorder : C.border}`,
        borderRadius: R.lg,
        padding: 16,
        display: "flex",
        flexDirection: "column",
        gap: 10,
        background: C.surface,
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <span style={{ fontSize: 11, padding: "3px 8px", borderRadius: "var(--sb-radius-pill)", background: C.brandSoft, color: C.brand }}>{a.platform}</span>
        <span style={{ display: "inline-block", fontSize: 11, color: C.muted, ...maskedText }}>{a.owner_type}</span>
        <div style={{ marginLeft: "auto", display: "flex", alignItems: "center", gap: 6, flexShrink: 0 }}>
          {online && (
            <span style={{ fontSize: 11, color: C.success, display: "flex", alignItems: "center", gap: 4, marginRight: 2 }}>
              <span aria-hidden="true" style={{ width: 6, height: 6, borderRadius: "var(--sb-radius-pill)", background: C.success }} />
              在线
            </span>
          )}
          {/* 不常用的、会删东西的收进菜单：每张卡摆一个红框「删除」太显眼，也容易误点 */}
          <div ref={menuRef} style={{ position: "relative" }} onKeyDown={(e) => e.key === "Escape" && setMenu(false)}>
            <IconButton
              label="更多操作"
              size="sm"
              onClick={() => setMenu(!menu)}
              haspopup="menu"
              expanded={menu}
            >
              <IconMore size={16} />
            </IconButton>
            {menu && (
              <div
                role="menu"
                className="sb-popover"
                style={{ position: "absolute", right: 0, top: 32, zIndex: 10, minWidth: 140, padding: 4 }}
              >
                {([
                  ["复制全部信息", () => void copyAccount(), C.text, "复制到剪贴板：含密码、TOTP 密钥和附加字段，要先验主密码"],
                  ["复制新建", () => p.onDuplicate(a), C.text, "以这个账号为模板新建一条"],
                  ["删除", () => p.onDelete(a), C.danger, "删除前还会再确认一次"],
                ] as const).map(([label, act, color, tip]) => (
                  <button
                    key={label}
                    type="button"
                    role="menuitem"
                    title={tip}
                    onClick={() => {
                      setMenu(false);
                      act();
                    }}
                    style={{ display: "block", width: "100%", textAlign: "left", padding: "7px 10px", border: 0, borderRadius: "var(--sb-radius-sm)", background: "transparent", fontSize: 12, color }}
                    onMouseEnter={(e) => (e.currentTarget.style.background = C.bg)}
                    onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
                  >
                    {label}
                  </button>
                ))}
              </div>
            )}
          </div>
        </div>
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
        <div style={{ fontSize: 14, color: C.text, fontWeight: 500, ...maskedText }}>{a.related_app || "（未填写应用）"}</div>
        <div style={{ fontSize: 12, color: C.sub, ...maskedText }}>
          {a.username} · {a.account_role}
        </div>
      </div>
      {/* 单独复制：账号库里存的明文只有这三样是平时要往别处粘的 */}
      <div style={{ display: "flex", alignItems: "center", gap: 6, flexWrap: "wrap" }}>
        <span
          role="status"
          title={copyStatus === "error" ? "复制失败，请检查剪贴板权限" : undefined}
          style={{ width: 46, flexShrink: 0, fontSize: 11, color: copyStatus === "done" ? C.success : copyStatus === "error" ? C.danger : C.muted }}
        >
          {copyLabel}
        </span>
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
              borderRadius: "var(--sb-radius-sm)",
              border: `1px solid ${C.border}`,
              background: C.surface,
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
      <div style={{ display: "flex", gap: 8, marginTop: 2 }}>
        <Button
          kind={a.login_url.trim() ? "primary" : "ghost"}
          size="sm"
          onClick={() => (a.login_url.trim() ? p.onLaunch(a) : p.onEdit(a))}
          title={blurred ? "打开账号页面" : a.login_url.trim() || "还没填登录直达 URL，点这里去补"}
        >
          {online ? "切过去" : a.login_url.trim() ? "一键登录" : "补登录地址"}
        </Button>
        {a.totp_secret && (
          <Button kind="ghost" size="sm" onClick={() => p.onTotp(a)}>
            验证码
          </Button>
        )}
        {/* 「手动填充」不单列了：「切过去」就是去会话页，填充按钮在那边 */}
        <Button kind="ghost" size="sm" onClick={() => p.onEdit(a)}>
          编辑
        </Button>
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
            borderRadius: "var(--sb-radius-lg)",
            border: `1px solid ${C.border}`,
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            fontSize: 24,
            color: C.brand,
          }}
        >
          <IconPlus size={24} />
        </div>
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <div style={{ fontSize: 20, color: C.text }}>账号库还是空的</div>
          <div style={{ fontSize: 13, color: C.sub, lineHeight: 1.6 }}>
            把阿里云、腾讯云、即构这些散落的账号登记进来
            <br />
            每条记录会自动获得独立的隔离登录环境
          </div>
        </div>
        <Button kind="primary" size="md" icon={<IconPlus size={15} />} onClick={onAdd}>
          添加第一个账号
        </Button>
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
        <Button kind="ghost" size="md" onClick={onAdd}>
          {masked ? "直接新增账号" : `直接新增“${keyword}”账号`}
        </Button>
      </div>
    </div>
  );
}
