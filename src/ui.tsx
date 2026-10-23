import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { clearClipboardLater, writeClipboard } from "./api";
import { IconAlert, IconCheck, IconChevronDown, IconClose } from "./icons";
import logo from "./assets/logo.svg";

/**
 * 图标 + 字的组合锁死在这里，只留一个字号旋钮，免得各处各调一套比例。
 * 三件事让它俩看着像一家的：
 * - 字用图标自己的深蓝 #323668，不用界面正文的近黑色
 * - 字重 600 对上图标的粗描边，否则细字被图标压住
 * - 图标 = 字号 × 1.5，line-height 归 1，让字的 cap-height 跟图标垂直居中对齐
 */
export function Brand({ size = 17 }: { size?: number }) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: Math.round(size * 0.48) }}>
      <img
        src={logo}
        width={Math.round(size * 1.5)}
        height={Math.round(size * 1.5)}
        alt=""
        style={{ display: "block", flexShrink: 0 }}
      />
      <span style={{ fontSize: size, lineHeight: 1, fontWeight: 600, letterSpacing: -0.2, color: C.ink }}>
        Switchboard
      </span>
    </div>
  );
}

/**
 * 调色板。值是 CSS 变量，浅色 / 深色两套定义在 index.html，跟着系统外观切换，组件里不用管。
 * 文字色都按 4.5:1 对比度挑过（浅色对白底和 bg，深色对 surface 和 bg）。
 * 危险色分两个：`danger` 给文字 / 描边，`dangerFill` 给需要白字的按钮（白字压它 5.6:1）。
 */
export const C = {
  /** 图标里的深蓝，品牌字用它，跟图标描边同色 */
  ink: "var(--sb-ink)",
  brand: "var(--sb-brand)",
  brandFill: "var(--sb-brand-fill)",
  brandSoft: "var(--sb-brand-soft)",
  text: "var(--sb-text)",
  sub: "var(--sb-sub)",
  muted: "var(--sb-muted)",
  border: "var(--sb-border)",
  borderStrong: "var(--sb-border-strong)",
  bg: "var(--sb-bg)",
  /** 卡片、输入框、弹层的底色：浅色下是白，深色下是深灰 */
  surface: "var(--sb-surface)",
  /** 品牌色 / 危险色按钮上的字，两种外观都是白 */
  onBrand: "#FFFFFF",
  danger: "var(--sb-danger)",
  dangerFill: "var(--sb-danger-fill)",
  dangerBorder: "var(--sb-danger-border)",
  dangerSoft: "var(--sb-danger-soft)",
  success: "var(--sb-success)",
  successBorder: "var(--sb-success-border)",
  warn: "var(--sb-warn)",
  warnSoft: "var(--sb-warn-soft)",
};

export const R = {
  xs: "var(--sb-radius-xs)",
  sm: "var(--sb-radius-sm)",
  md: "var(--sb-radius-md)",
  lg: "var(--sb-radius-lg)",
  xl: "var(--sb-radius-xl)",
} as const;

export const inputStyle: CSSProperties = {
  width: "100%",
  boxSizing: "border-box",
  padding: "10px 12px",
  borderRadius: R.sm,
  border: `1px solid ${C.borderStrong}`,
  background: C.surface,
  fontSize: 14,
  color: C.text,
};

/**
 * 可搜索候选 + 自由输入的组合框，对应原型的 FieldCandidates：
 * 已经存过的值直接选，没有的直接输入即新建，不需要额外的"新建"按钮。
 * 候选来自「内置默认值 ∪ 账号库里已经用过的值」，所以新值保存一次就进候选，
 * 不需要单独存一张选项表。
 */
export function Combo({
  value,
  onChange,
  options,
  placeholder,
}: {
  value: string;
  onChange: (v: string) => void;
  options: string[];
  placeholder?: string;
}) {
  const [open, setOpen] = useState(false);
  const [cursor, setCursor] = useState(0);
  const kw = value.trim().toLowerCase();
  const hits = options.filter((o) => o.toLowerCase().includes(kw));
  const isNew = value.trim() !== "" && !options.includes(value.trim());

  const pick = (v: string) => {
    onChange(v);
    setOpen(false);
  };

  return (
    <div style={{ position: "relative" }}>
      <input
        value={value}
        placeholder={placeholder}
        role="combobox"
        aria-expanded={open}
        aria-autocomplete="list"
        onChange={(e) => {
          onChange(e.target.value);
          setCursor(0);
          setOpen(true);
        }}
        onFocus={() => setOpen(true)}
        onBlur={() => setOpen(false)}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown") {
            e.preventDefault();
            setOpen(true);
            setCursor((c) => Math.min(c + 1, Math.max(hits.length - 1, 0)));
          } else if (e.key === "ArrowUp") {
            e.preventDefault();
            setCursor((c) => Math.max(c - 1, 0));
          } else if (e.key === "Enter" && open && hits[cursor]) {
            e.preventDefault();
            pick(hits[cursor]);
          } else if (e.key === "Escape") {
            setOpen(false);
          }
        }}
        style={{ ...inputStyle, paddingRight: 28 }}
      />
      <span style={{ position: "absolute", right: 10, top: "50%", transform: "translateY(-50%)", color: C.muted, pointerEvents: "none" }}>
        <IconChevronDown size={12} />
      </span>

      {open && (hits.length > 0 || isNew) && (
        <div
          role="listbox"
          className="sb-popover"
          style={{
            position: "absolute",
            zIndex: 20,
            top: "calc(100% + 4px)",
            left: 0,
            right: 0,
            maxHeight: 220,
            overflowY: "auto",
            padding: 4,
          }}
        >
          {hits.map((o, i) => (
            <button
              key={o}
              type="button"
              role="option"
              aria-selected={i === cursor}
              // mousedown 早于 input 的 blur，否则点不中
              onMouseDown={(e) => {
                e.preventDefault();
                pick(o);
              }}
              onMouseEnter={() => setCursor(i)}
              style={{
                display: "block",
                width: "100%",
                textAlign: "left",
                padding: "8px 10px",
                border: 0,
                borderRadius: R.sm,
                background: i === cursor ? C.bg : "transparent",
                fontSize: 13,
                color: C.text,
                cursor: "pointer",
              }}
            >
              {o}
            </button>
          ))}
          {isNew && (
            <div style={{ padding: "8px 10px", fontSize: 12, color: C.brand }}>
              新值「{value.trim()}」，保存后自动进入候选
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export function Field({ label, hint, children }: { label: string; hint?: ReactNode; children: ReactNode }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      <label style={{ fontSize: 12, color: C.sub }}>{label}</label>
      {children}
      {hint && <div style={{ fontSize: 11, color: C.muted, lineHeight: 1.7 }}>{hint}</div>}
    </div>
  );
}

type ButtonKind = "primary" | "ghost" | "danger" | "subtle" | "dangerGhost";
type ButtonSize = "sm" | "md" | "lg";

const KIND_CLASS: Record<ButtonKind, string> = {
  primary: "sb-btn-primary",
  ghost: "sb-btn-ghost",
  danger: "sb-btn-danger",
  subtle: "sb-btn-subtle",
  dangerGhost: "sb-btn-danger-ghost",
};

type BtnProps = {
  children: ReactNode;
  onClick?: () => void;
  kind?: ButtonKind;
  /** 尺寸：sm=30 / md=36 / lg=44；不传按 md */
  size?: ButtonSize;
  disabled?: boolean;
  loading?: boolean;
  icon?: ReactNode;
  style?: CSSProperties;
  title?: string;
  type?: "button" | "submit";
  "aria-label"?: string;
};

export function Button({
  children,
  onClick,
  kind = "ghost",
  size,
  disabled,
  loading,
  icon,
  style,
  title,
  type = "button",
  ...rest
}: BtnProps) {
  const sizeClass = `sb-btn-${size ?? "md"}`;
  return (
    <button
      type={type}
      onClick={onClick}
      disabled={disabled || loading}
      title={title}
      aria-busy={loading || undefined}
      className={`sb-btn ${sizeClass} ${KIND_CLASS[kind]}`}
      style={style}
      {...rest}
    >
      {loading ? <Spinner /> : icon}
      {children}
    </button>
  );
}

/** 只有图标、必须带 aria-label 的按钮（工具栏、页签、卡片菜单） */
export function IconButton({
  label,
  onClick,
  children,
  size = "md",
  pressed,
  expanded,
  haspopup,
  disabled,
  title,
  style,
}: {
  label: string;
  onClick?: () => void;
  children: ReactNode;
  size?: "md" | "sm" | "xs";
  pressed?: boolean;
  expanded?: boolean;
  haspopup?: boolean | "menu";
  disabled?: boolean;
  title?: string;
  style?: CSSProperties;
}) {
  const sizeClass = size === "sm" ? "sb-iconbtn-sm" : size === "xs" ? "sb-iconbtn-xs" : "";
  return (
    <button
      type="button"
      aria-label={label}
      aria-pressed={pressed}
      aria-expanded={expanded}
      aria-haspopup={haspopup}
      title={title ?? label}
      disabled={disabled}
      onClick={onClick}
      className={`sb-iconbtn ${sizeClass}`}
      style={style}
    >
      {children}
    </button>
  );
}

/**
 * 整行可点的容器（账号行、快捷切换项、导航项）。
 * 用 role=button + 键盘处理，而不是裸 div，Tab 能走到、Enter/Space 能触发。
 */
export function Pressable({
  children,
  onClick,
  onMouseEnter,
  style,
  title,
  label,
  disabled,
  className = "",
}: {
  children: ReactNode;
  onClick?: () => void;
  onMouseEnter?: () => void;
  style?: CSSProperties;
  title?: string;
  label?: string;
  disabled?: boolean;
  className?: string;
}) {
  return (
    <div
      role="button"
      tabIndex={disabled ? -1 : 0}
      aria-disabled={disabled || undefined}
      aria-label={label}
      title={title}
      className={`sb-pressable ${className}`.trim()}
      onClick={disabled ? undefined : onClick}
      onMouseEnter={onMouseEnter}
      onKeyDown={(e) => {
        if (disabled) return;
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onClick?.();
        }
      }}
      style={style}
    >
      {children}
    </div>
  );
}

/** 小圆问号。标题旁边那种，点开才显示说明，平时不占地方。 */
export function HelpDot({ onClick, title, on = false }: { onClick: () => void; title?: string; on?: boolean }) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={title}
      aria-label={title ?? "帮助"}
      aria-expanded={on}
      style={{
        width: 22,
        height: 22,
        flexShrink: 0,
        padding: 0,
        borderRadius: "var(--sb-radius-pill)",
        border: `1px solid ${on ? C.brand : C.border}`,
        background: on ? C.brandSoft : "transparent",
        color: on ? C.brand : C.sub,
        fontSize: 12,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        cursor: "pointer",
      }}
    >
      ?
    </button>
  );
}

export function Card({ children, style }: { children: ReactNode; style?: CSSProperties }) {
  return (
    <div
      style={{
        background: C.surface,
        border: `1px solid ${C.border}`,
        borderRadius: R.lg,
        padding: 24,
        ...style,
      }}
    >
      {children}
    </div>
  );
}

/** 行内转圈。按钮 loading 时替掉前面的图标 */
export function Spinner({ size = 14 }: { size?: number }) {
  return (
    <span
      aria-hidden="true"
      style={{
        width: size,
        height: size,
        flexShrink: 0,
        display: "inline-block",
        borderRadius: "50%",
        border: "2px solid currentColor",
        borderTopColor: "transparent",
        animation: "sb-spin 700ms linear infinite",
      }}
    />
  );
}

/** 首屏 / 列表加载占位 */
export function Skeleton({ width = "100%", height = 12, style }: { width?: number | string; height?: number; style?: CSSProperties }) {
  return <div className="sb-skeleton" aria-hidden="true" style={{ width, height, ...style }} />;
}

/**
 * 统一的弹层：portal 到 body、焦点锁在框里、Esc 关闭、关闭后焦点还给触发按钮。
 * 以前每个弹层各写一遍 fixed + 遮罩，键盘用户打开后就找不到焦点了。
 */
export function Dialog({
  children,
  onClose,
  label,
  width = 380,
  align = "center",
  dismissible = true,
  style,
}: {
  children: ReactNode;
  onClose: () => void;
  label: string;
  width?: number | string;
  align?: "center" | "top";
  /** 网络告警这类必须走明确出口的框：Esc / 点遮罩都不关 */
  dismissible?: boolean;
  style?: CSSProperties;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    const el = ref.current;
    const focusables = () =>
      Array.from(
        el?.querySelectorAll<HTMLElement>(
          'a[href], button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])',
        ) ?? [],
      ).filter((node) => node.offsetParent !== null || node === document.activeElement);

    // 内容里自己写了 autoFocus 就尊重它，否则把焦点送进第一个可操作项
    if (el && !el.contains(document.activeElement)) {
      const first = focusables()[0];
      (first ?? el).focus();
    }

    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && dismissible) {
        e.preventDefault();
        // 捕获阶段拦下来，别让 App 那层的全局 Esc 再处理一次
        e.stopPropagation();
        onCloseRef.current();
        return;
      }
      if (e.key !== "Tab" || !el) return;
      const list = focusables();
      if (list.length === 0) {
        e.preventDefault();
        el.focus();
        return;
      }
      const first = list[0];
      const last = list[list.length - 1];
      const active = document.activeElement;
      if (!el.contains(active)) {
        e.preventDefault();
        first.focus();
      } else if (e.shiftKey && (active === first || active === el)) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && active === last) {
        e.preventDefault();
        first.focus();
      }
    };

    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    document.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("keydown", onKey, true);
      document.body.style.overflow = previousOverflow;
      if (previous?.isConnected) previous.focus();
    };
  }, []);

  return createPortal(
    <div
      className="sb-dialog-backdrop"
      data-align={align}
      onClick={(e) => {
        if (e.target === e.currentTarget && dismissible) onClose();
      }}
    >
      <div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-label={label}
        tabIndex={-1}
        className="sb-dialog"
        style={{ width, ...style }}
      >
        {children}
      </div>
    </div>,
    document.body,
  );
}

/** 顶部/底部提示条。tone=error 用 alert，其余用 status，读屏能播报 */
export function Toast({
  children,
  onClose,
  tone = "error",
  style,
}: {
  children: ReactNode;
  onClose: () => void;
  tone?: "error" | "info" | "success";
  style?: CSSProperties;
}) {
  const color = tone === "error" ? C.danger : tone === "success" ? C.success : C.brand;
  const Icon = tone === "success" ? IconCheck : IconAlert;
  return (
    <div
      className="sb-toast"
      role={tone === "error" ? "alert" : "status"}
      aria-live={tone === "error" ? "assertive" : "polite"}
      style={{ borderColor: tone === "error" ? C.dangerBorder : undefined, ...style }}
    >
      <span style={{ color, flexShrink: 0, marginTop: 2 }}><Icon size={15} /></span>
      <span style={{ flex: 1, minWidth: 0, color: C.sub, overflowWrap: "anywhere" }}>{children}</span>
      <IconButton label="关闭提示" size="xs" onClick={onClose} style={{ marginTop: -2 }}>
        <IconClose size={12} />
      </IconButton>
    </div>
  );
}

/**
 * 复制到剪贴板。WKWebView 里 `navigator.clipboard`（execCommand 也一样）只认用户点击的当下，
 * 而复制密码之前要 await 验主密码、取明文，手势早过期了——所以写不进去就交给后端写。
 */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return writeClipboard(text).then(() => true, () => false);
  }
}

/**
 * 跑在哪个系统上。WKWebView 的 UA 带 "Macintosh"，WebView2 带 "Windows NT"。
 * 只用来切界面文案、藏 macOS 专属功能（录屏、识别二维码图片），不参与任何安全判断
 */
export const IS_MAC = /Macintosh/.test(navigator.userAgent);
/** 快捷键修饰键的写法：⌘K / Ctrl+K */
export const MOD = IS_MAC ? "⌘" : "Ctrl+";
export const FILE_MANAGER = IS_MAC ? "访达" : "资源管理器";

/** 复制账号、密码这类明文：30 秒后后端把剪贴板清掉（剪贴板那时还是这段才清） */
export async function copySecret(text: string): Promise<boolean> {
  const ok = await copyText(text);
  if (ok) void clearClipboardLater(text).catch(() => {});
  return ok;
}

export function ErrorLine({ text }: { text: string }) {
  if (!text) return null;
  return <div role="alert" style={{ fontSize: 13, color: C.danger, lineHeight: 1.6 }}>{text}</div>;
}
