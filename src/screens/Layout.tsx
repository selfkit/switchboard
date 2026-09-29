import { useState, type ReactNode } from "react";
import { Brand, C, HelpDot } from "../ui";
import { DEMO_PASSWORD, type RegionState } from "../api";
import { RegionBadge } from "./RegionGuard";

/**
 * 顶栏高度。**必须和 session.rs 的 TOPBAR_H 一致** —— 账号页面那个原生 webview
 * 正好从它下面开始铺（session.rs 的 frontend_layout_constants_match 钉着这两个数）。
 */
export const TOPBAR_H = 40;
/** 顶栏下面那条页签条的高度，同样和 session.rs 的 TABS_H 对齐 */
export const TABS_H = 34;

/** 主界面的几个去处。账号列表页还要在这份导航上面插"归属/平台"筛选，所以只共用底部这段。 */
export type Page = "main" | "sessions" | "settings" | "platforms";

/**
 * 全局顶栏：整个 App 只有这一条，固定在最上面，品牌标钉在右上角。
 * 高度必须和 session.rs 的 TOPBAR_H 对上——账号页面那个原生 webview 正好从它下面开始铺。
 */
export function AppShell({ children, demo, tour, onStopTour, region, checking, notice, onCheck, onDismiss }: {
  children: ReactNode;
  demo: boolean;
  /** 一键自动演示的解说字幕，居中显示，录屏时就是视频里的字幕 */
  tour: { text: string; running: boolean } | null;
  onStopTour: () => void;
  region: RegionState | null;
  checking: boolean;
  notice: string;
  onCheck: () => void;
  onDismiss: () => void;
}) {
  return (
    <div style={{ height: "100vh", display: "flex", flexDirection: "column" }}>
      <div
        data-tauri-drag-region="deep"
        style={{
          height: TOPBAR_H,
          flexShrink: 0,
          display: "flex",
          alignItems: "center",
          justifyContent: "flex-end",
          gap: 10,
          padding: "0 20px",
          boxSizing: "border-box",
          position: "relative",
        }}
      >
        {/* 提醒演示的人自己：现在是假库。主密码写出来，复制明文、改设置时要输 */}
        {tour && (
          <div data-tauri-drag-region="false" style={{ position: "absolute", left: "50%", transform: "translateX(-50%)", display: "flex", alignItems: "center", gap: 10, padding: "4px 14px", borderRadius: 999, background: C.brandSoft, color: C.brand, fontSize: 13, whiteSpace: "nowrap" }}>
            {tour.running && "▶ "}
            {tour.text}
            {tour.running && (
              <span onClick={onStopTour} title="停止自动演示；正在录的屏会丢掉" style={{ fontSize: 11, color: C.muted, cursor: "pointer", textDecoration: "underline" }}>
                停止
              </span>
            )}
          </div>
        )}
        {/* 自动演示时字幕替掉它：视频里不需要这行给演示者看的提醒 */}
        {demo && !tour?.running && (
          <span title="锁定账号库即退出演示，回到真实账号库" style={{ padding: "2px 10px", borderRadius: 999, background: "rgba(255,125,0,0.12)", color: C.warn, fontSize: 11 }}>
            演示模式 · 全是假数据 · 主密码 {DEMO_PASSWORD} · 锁定即退出
          </span>
        )}
        <RegionBadge state={region} busy={checking} onClick={onCheck} />
        <Brand size={15} />
        {notice && (
          <div data-tauri-drag-region="false" style={{ position: "absolute", top: 42, right: 20, zIndex: 35, maxWidth: 330, padding: "12px 14px", borderRadius: 8, background: C.surface, border: `1px solid ${C.border}`, boxShadow: "0 4px 18px rgba(0,0,0,0.12)", fontSize: 12, color: C.sub, lineHeight: 1.7 }}>
            {notice}
            <span onClick={onDismiss} style={{ marginLeft: 10, color: C.brand, cursor: "pointer" }}>关闭</span>
          </div>
        )}
      </div>
      {/* 横向 flex：页面各自用 flexGrow 撑开，会话页那条侧栏则保持 224 固定宽 */}
      <div style={{ flexGrow: 1, minHeight: 0, display: "flex", alignItems: "stretch" }}>{children}</div>
    </div>
  );
}

export function NavItem({
  label,
  count,
  active,
  muted,
  compact,
  blurLabel,
  onClick,
}: {
  label: string;
  count?: number | null;
  active?: boolean;
  muted?: boolean;
  compact?: boolean;
  blurLabel?: boolean;
  onClick: () => void;
}) {
  return (
    <div
      onClick={onClick}
      onMouseEnter={(e) => {
        if (!active) e.currentTarget.style.background = "rgba(0,0,0,0.035)";
      }}
      onMouseLeave={(e) => {
        if (!active) e.currentTarget.style.background = "transparent";
      }}
      style={{
        padding: compact ? "6px 10px" : "9px 10px",
        borderRadius: 7,
        background: active ? C.surface : "transparent",
        // 选中态用描边+阴影代替"忽然变白"，切换时不刺眼
        boxShadow: active ? "0 1px 2px rgba(0,0,0,0.05)" : "none",
        fontSize: muted ? 12 : 13,
        color: active ? C.text : muted ? C.muted : C.sub,
        fontWeight: active ? 500 : 400,
        display: "flex",
        justifyContent: "space-between",
        alignItems: "center",
        gap: 8,
        cursor: "pointer",
        transition: "background 120ms ease, box-shadow 120ms ease",
        userSelect: "none",
      }}
    >
      <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", filter: blurLabel ? "blur(7px)" : "none", userSelect: blurLabel ? "none" : "auto" }}>{label}</span>
      {count ? <span style={{ color: C.muted, flexShrink: 0, fontSize: 12 }}>{count}</span> : null}
    </div>
  );
}

/** 三个页面底部共用的那段导航，只此一份 */
/**
 * 三个页面底部共用的那段导航，只此一份。放在 Sidebar 的 footer 槽里，
 * **永远钉在侧栏底部、不跟着上面的内容滚**：
 * - 之前设置页只有它一项所以贴顶，从账号列表点过去整块会从底部跳到顶部，很累
 * - 之前整条侧栏一起滚，账号一多这段就被顶出可视区，要滚到底才点得到
 */
export function SideNav({
  current,
  sessionCount,
  onGo,
  onLock,
}: {
  current: Page;
  sessionCount: number;
  onGo: (p: Page) => void;
  /** 每一页都要传：少一项的话导航块高度会变，切页时上边缘还是会跳 */
  onLock: () => void;
}) {
  return (
    <div style={{ flexShrink: 0, display: "flex", flexDirection: "column", gap: 2, paddingTop: 12, borderTop: `1px solid ${C.border}` }}>
      <NavItem label="账号列表" active={current === "main"} onClick={() => onGo("main")} />
      <NavItem label="会话" count={sessionCount} active={current === "sessions"} onClick={() => onGo("sessions")} />
      <NavItem label="设置" active={current === "settings"} onClick={() => onGo("settings")} />
      <NavItem label="平台适配配置" active={current === "platforms"} onClick={() => onGo("platforms")} />
      <NavItem label="锁定账号库" muted onClick={onLock} />
    </div>
  );
}

/**
 * 可折叠的筛选分组。账号一多，「归属」和「平台」两组能把侧栏撑到要滚半天，
 * 所以三件事一起上：
 * - 点标题整组收起，状态记在 localStorage，切页不重置
 * - 组内按数量从多到少排，常用的永远在最上面
 * - 超过 `max` 条就折起来，点"还有 N 个"展开；当前选中的那条一定可见，
 *   否则选了个冷门平台再收起来，会找不到自己选中的是哪个
 */
export function FilterGroup({
  title,
  items,
  activeValue,
  onPick,
  blurValues,
  max = 4,
}: {
  title: string;
  items: [string, number][];
  activeValue: string | null;
  onPick: (v: string) => void;
  blurValues?: boolean;
  max?: number;
}) {
  const key = `sb.nav.${title}.collapsed`;
  const [collapsed, setCollapsed] = useState(() => localStorage.getItem(key) === "1");
  const [expanded, setExpanded] = useState(false);

  const sorted = [...items].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  let shown = expanded || sorted.length <= max ? sorted : sorted.slice(0, max);
  if (activeValue && !shown.some(([v]) => v === activeValue)) {
    const hit = sorted.find(([v]) => v === activeValue);
    if (hit) shown = [...shown, hit];
  }
  const hiddenCount = sorted.length - shown.length;

  const toggle = () => {
    const next = !collapsed;
    setCollapsed(next);
    localStorage.setItem(key, next ? "1" : "0");
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
      <button
        type="button"
        aria-expanded={!collapsed}
        onClick={toggle}
        style={{
          width: "100%",
          border: 0,
          background: "transparent",
          textAlign: "left",
          fontSize: 11,
          color: C.muted,
          padding: "0 8px 6px",
          display: "flex",
          alignItems: "center",
          gap: 5,
          cursor: "pointer",
          userSelect: "none",
        }}
      >
        <span style={{ display: "inline-block", transform: collapsed ? "rotate(-90deg)" : "none", transition: "transform 120ms ease", fontSize: 9 }}>
          ▾
        </span>
        {title}
        <span style={{ marginLeft: "auto" }}>{items.length}</span>
      </button>

      {!collapsed && (
        <>
          {shown.map(([v, n]) => (
            <NavItem key={v} label={v} count={n} active={activeValue === v} compact blurLabel={blurValues} onClick={() => onPick(v)} />
          ))}
          {(hiddenCount > 0 || expanded && sorted.length > max) && (
            <button
              type="button"
              onClick={() => setExpanded(!expanded)}
              style={{ padding: "6px 10px", border: 0, background: "transparent", textAlign: "left", fontSize: 11, color: C.brand, cursor: "pointer", userSelect: "none" }}
            >
              {expanded ? "收起" : `显示其余 ${hiddenCount} 项`}
            </button>
          )}
        </>
      )}
    </div>
  );
}

/**
 * 左侧 224px 灰底栏的外壳，三个页面尺寸/底色要一致。
 * 只有上半部分滚动，`footer`（导航）固定在底部。
 */
export function Sidebar({ children, footer }: { children?: ReactNode; footer?: ReactNode }) {
  return (
    <div
      style={{
        width: 224,
        flexShrink: 0,
        boxSizing: "border-box",
        padding: "20px 16px 16px",
        background: C.bg,
        borderRight: `1px solid ${C.border}`,
        display: "flex",
        flexDirection: "column",
      }}
    >
      <div style={{ flexGrow: 1, minHeight: 0, overflowY: "auto", display: "flex", flexDirection: "column", gap: 18 }}>
        {children}
      </div>
      {footer}
    </div>
  );
}

/** 侧栏上半截的一组：小标题 + 一列条目。账号列表页的「归属 / 平台」是能折叠的 FilterGroup，这个是固定的 */
export function SideGroup({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
      <div style={{ fontSize: 11, color: C.muted, padding: "0 8px 6px" }}>{title}</div>
      {children}
    </div>
  );
}

/** App 交给设置页 / 平台适配配置页的那几样，页面自己拿去搭 SubPage */
export type SubPageShell = {
  sessionCount: number;
  onGo: (p: Page) => void;
  onLock: () => void;
  onHelp: () => void;
};

/**
 * 设置 / 平台适配配置共用的壳，对应 Settings.dc.html 与 PlatformConfig.dc.html。
 * 侧栏上半截（`side`）放这一页自己的导航——设置的分区、平台配置的状态筛选，跟账号列表页的筛选一个路数；
 * 不放的话侧栏只剩底下那段全局导航，上面一大片空着
 */
export function SubPage({
  current,
  sessionCount,
  onGo,
  onLock,
  onHelp,
  side,
  children,
}: SubPageShell & {
  current: Page;
  side?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div style={{ flexGrow: 1, minWidth: 0, display: "flex", background: C.surface }}>
      <Sidebar footer={<SideNav current={current} sessionCount={sessionCount} onGo={onGo} onLock={onLock} />}>{side}</Sidebar>
      <div style={{ flexGrow: 1, overflowY: "auto", padding: "24px 48px 40px", position: "relative" }}>
        {/* 这个问号讲的是"新平台怎么适配"，只跟平台适配配置页有关，设置页不该出现 */}
        {current === "platforms" && (
          <div style={{ position: "absolute", top: 26, right: 48 }}>
            <HelpDot onClick={onHelp} title="新平台怎么适配（不用重新打包）" />
          </div>
        )}
        {children}
      </div>
    </div>
  );
}
