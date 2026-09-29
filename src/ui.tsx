import { useState, type CSSProperties, type ReactNode } from "react";
import { clearClipboardLater, writeClipboard } from "./api";
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
 * 文字色都按 4.5:1 对比度挑过（浅色对白底和 bg，深色对 surface 和 bg）
 */
export const C = {
  /** 图标里的深蓝，品牌字用它，跟图标描边同色 */
  ink: "var(--sb-ink)",
  brand: "var(--sb-brand)",
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
  success: "var(--sb-success)",
  warn: "var(--sb-warn)",
};

export const inputStyle: CSSProperties = {
  width: "100%",
  boxSizing: "border-box",
  padding: "10px 12px",
  borderRadius: 7,
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
  const kw = value.trim().toLowerCase();
  const hits = options.filter((o) => o.toLowerCase().includes(kw));
  const isNew = value.trim() !== "" && !options.includes(value.trim());

  return (
    <div style={{ position: "relative" }}>
      <input
        value={value}
        placeholder={placeholder}
        onChange={(e) => {
          onChange(e.target.value);
          setOpen(true);
        }}
        onFocus={() => setOpen(true)}
        onBlur={() => setOpen(false)}
        style={{ ...inputStyle, paddingRight: 28 }}
      />
      <span style={{ position: "absolute", right: 10, top: 12, fontSize: 11, color: C.muted, pointerEvents: "none" }}>▾</span>

      {open && (hits.length > 0 || isNew) && (
        <div
          style={{
            position: "absolute",
            zIndex: 20,
            top: "calc(100% + 4px)",
            left: 0,
            right: 0,
            maxHeight: 220,
            overflowY: "auto",
            background: C.surface,
            border: `1px solid ${C.border}`,
            borderRadius: 8,
            boxShadow: "0 6px 20px rgba(0,0,0,0.10)",
            padding: 4,
          }}
        >
          {hits.map((o) => (
            <div
              key={o}
              // mousedown 早于 input 的 blur，否则点不中
              onMouseDown={(e) => {
                e.preventDefault();
                onChange(o);
                setOpen(false);
              }}
              style={{ padding: "8px 10px", borderRadius: 6, fontSize: 13, color: C.text, cursor: "pointer" }}
              onMouseEnter={(e) => (e.currentTarget.style.background = C.bg)}
              onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
            >
              {o}
            </div>
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

type BtnProps = {
  children: ReactNode;
  onClick?: () => void;
  kind?: "primary" | "ghost" | "danger";
  disabled?: boolean;
  style?: CSSProperties;
  type?: "button" | "submit";
};

export function Button({ children, onClick, kind = "ghost", disabled, style, type = "button" }: BtnProps) {
  const skin: CSSProperties =
    kind === "primary"
      ? { background: C.brand, color: C.onBrand, border: "none", fontWeight: 500 }
      : kind === "danger"
        ? { background: C.danger, color: C.onBrand, border: "none", fontWeight: 500 }
        : { background: C.surface, color: C.sub, border: `1px solid ${C.borderStrong}` };
  return (
    <button
      type={type}
      onClick={onClick}
      disabled={disabled}
      style={{
        padding: "10px 18px",
        borderRadius: 7,
        fontSize: 14,
        opacity: disabled ? 0.5 : 1,
        cursor: disabled ? "default" : "pointer",
        ...skin,
        ...style,
      }}
    >
      {children}
    </button>
  );
}

/** 小圆问号。标题旁边那种，点开才显示说明，平时不占地方。 */
export function HelpDot({ onClick, title, on = false }: { onClick: () => void; title?: string; on?: boolean }) {
  return (
    <div
      onClick={onClick}
      title={title}
      style={{
        width: 22,
        height: 22,
        flexShrink: 0,
        borderRadius: 999,
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
    </div>
  );
}

export function Card({ children, style }: { children: ReactNode; style?: CSSProperties }) {
  return (
    <div
      style={{
        background: C.surface,
        border: `1px solid ${C.border}`,
        borderRadius: 12,
        padding: 24,
        ...style,
      }}
    >
      {children}
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
  return <div style={{ fontSize: 13, color: C.danger }}>{text}</div>;
}
