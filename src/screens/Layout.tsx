import { useEffect, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Brand, C, HelpDot, IS_MAC } from "../ui";
import { DEMO_PASSWORD, type RegionState } from "../api";
import { RegionBadge } from "./RegionGuard";

/**
 * 顶栏高度。**必须和 session.rs 的 TOPBAR_H 一致** —— 账号页面那个原生 webview
 * 正好从它下面开始铺（session.rs 的 frontend_layout_constants_match 钉着这两个数）。
 */
export const TOPBAR_H = 52;
/** 顶栏下面那条页签条的高度，同样和 session.rs 的 TABS_H 对齐 */
export const TABS_H = 34;
/** 侧栏宽度，同样和 session.rs 的 SIDEBAR_W 对齐 */
export const SIDEBAR_W = 224;

/** 顶栏中间那块留给页面自己放东西（账号列表的搜索和按钮），见 `TopbarSlot` */
const TOPBAR_SLOT_ID = "sb-topbar-slot";

/**
 * 把页面自己的工具栏放进顶栏，而不是在顶栏下面再起一行：两行叠着，上面那行只有右上角一个标，
 * 大半截空着。用 portal 是为了状态（搜索词、模糊开关）还留在页面里，不用往 App 上提
 */
export function TopbarSlot({ children }: { children: ReactNode }) {
  // 顶栏和页面同一次提交挂上去，页面第一次渲染时槽位还没进 DOM，所以挂载后再取
  const [el, setEl] = useState<HTMLElement | null>(null);
  useEffect(() => setEl(document.getElementById(TOPBAR_SLOT_ID)), []);
  return el ? createPortal(children, el) : null;
}

/** 主界面的几个去处。账号列表页还要在这份导航上面插"归属/平台"筛选，所以只共用底部这段。 */
export type Page = "main" | "sessions" | "settings" | "platforms";

/**
 * 全局顶栏：整个 App 只有这一条，固定在最上面，品牌标钉在右上角。
 * 高度必须和 session.rs 的 TOPBAR_H 对上——账号页面那个原生 webview 正好从它下面开始铺。
 */
export function AppShell({ children, split, demo, tour, onStopTour, region, checking, notice, onCheck, onDismiss }: {
  children: ReactNode;
  /**
   * 下面是「侧栏 + 内容」的页面：侧栏和内容同一个底色，只靠一条细线分开，这条线从窗口顶通到底。
   * 试过两种不行的：顶栏整条灰、内容白——内容区上沿一道硬边，分隔线从半截开始；
   * 侧栏灰、内容白各占一列——两块色从上到下一刀切开，割裂感更重。Windows 的原生标题栏是白的，同底色也最不突兀
   */
  split: boolean;
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
          // 侧栏页左边那列要跟下面的侧栏严丝合缝，不留内边距
          padding: split ? "0 20px 0 0" : "0 20px",
          boxSizing: "border-box",
          position: "relative",
          // 分隔线那 1px 对齐 Sidebar 的 borderRight（宽 SIDEBAR_W、border-box，线在最右一像素）
          background: split
            ? `linear-gradient(to right, ${C.surface} ${SIDEBAR_W - 1}px, ${C.border} ${SIDEBAR_W - 1}px ${SIDEBAR_W}px, ${C.surface} ${SIDEBAR_W}px)`
            : "transparent",
        }}
      >
        {/* 侧栏那一列的顶上放品牌，像侧栏的标题；macOS 左上角有红绿灯，让开它 */}
        {split && (
          // marginRight 抵掉顶栏的 gap：槽位要正好从侧栏右边线开始，页面里的东西才跟下面的内容对齐
          <div style={{ width: SIDEBAR_W, marginRight: -10, flexShrink: 0, alignSelf: "stretch", display: "flex", alignItems: "center", boxSizing: "border-box", paddingLeft: IS_MAC ? 84 : 16 }}>
            <Brand size={14} />
          </div>
        )}
        {/* 页面的工具栏放这里（TopbarSlot）。自动演示时字幕占顶栏正中，先把它藏起来免得叠在一起 */}
        <div id={TOPBAR_SLOT_ID} style={{ flexGrow: 1, minWidth: 0, alignSelf: "stretch", display: "flex", alignItems: "center", visibility: tour ? "hidden" : "visible" }} />
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
          <span title="全是假数据，不碰真实账号库；锁定账号库即退出演示" style={{ padding: "2px 10px", borderRadius: 999, background: "rgba(255,125,0,0.12)", color: C.warn, fontSize: 11, whiteSpace: "nowrap", flexShrink: 0 }}>
            演示模式 · 主密码 {DEMO_PASSWORD}
          </span>
        )}
        <RegionBadge state={region} busy={checking} onClick={onCheck} />
        {!split && <Brand size={15} />}
        {notice && (
          <div data-tauri-drag-region="false" style={{ position: "absolute", top: TOPBAR_H + 2, right: 20, zIndex: 35, maxWidth: 330, padding: "12px 14px", borderRadius: 8, background: C.surface, border: `1px solid ${C.border}`, boxShadow: "0 4px 18px rgba(0,0,0,0.12)", fontSize: 12, color: C.sub, lineHeight: 1.7 }}>
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
        // 侧栏是白底，选中的用浅灰底标出来
        background: active ? C.bg : "transparent",
        fontSize: muted ? 12 : 13,
        color: active ? C.text : muted ? C.muted : C.sub,
        fontWeight: active ? 500 : 400,
        display: "flex",
        justifyContent: "space-between",
        alignItems: "center",
        gap: 8,
        cursor: "pointer",
        transition: "background 120ms ease",
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
 * 左侧 224px 侧栏的外壳，三个页面尺寸/底色要一致。跟内容区同底色，只靠右边一条细线分开
 * 只有上半部分滚动，`footer`（导航）固定在底部。
 */
export function Sidebar({ children, footer }: { children?: ReactNode; footer?: ReactNode }) {
  return (
    <div
      style={{
        width: SIDEBAR_W,
        flexShrink: 0,
        boxSizing: "border-box",
        padding: "20px 16px 16px",
        background: C.surface,
        borderRight: `1px solid ${C.border}`,
        display: "flex",
        flexDirection: "column",
      }}
    >
      {/* sb-scroll：滚动条只在鼠标移上来时出现（index.html），平时贴着分隔线的那道灰条会把线加粗一倍 */}
      <div className="sb-scroll" style={{ flexGrow: 1, minHeight: 0, overflowY: "auto", display: "flex", flexDirection: "column", gap: 18 }}>
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
