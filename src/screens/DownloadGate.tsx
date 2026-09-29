import { C } from "../ui";

/** 发行版仓库。更新检查和下载都指向这里。 */
export const RELEASE_PAGE = "https://gitee.com/etn/switchboard_store/releases";

/**
 * 这份前端是给桌面 App 用的，不是网页。
 * 被人用普通浏览器打开时（本地起了 dev server、或者有人把 dist 传到某个静态站），
 * 所有 invoke 都会失败、界面只会一直转圈或报一堆看不懂的错。
 * 与其那样，不如直接告诉他去哪下客户端。
 */
export default function DownloadGate() {
  const box: React.CSSProperties = {
    width: 460,
    background: C.surface,
    border: `1px solid ${C.border}`,
    borderRadius: 14,
    padding: 32,
    display: "flex",
    flexDirection: "column",
    gap: 16,
    alignItems: "center",
    textAlign: "center",
  };

  return (
    <div style={{ minHeight: "100vh", display: "flex", alignItems: "center", justifyContent: "center", padding: 24, background: C.bg }}>
      <div style={box}>
        <div style={{ fontSize: 20, fontWeight: 600, color: C.ink }}>Switchboard 是桌面应用</div>
        <div style={{ fontSize: 13, color: C.sub, lineHeight: 1.8 }}>
          它要直接操作本机的加密账号库和隔离的浏览器会话，这些能力浏览器里没有，所以网页版打不开。
          请下载桌面客户端使用。
        </div>
        <a
          href={RELEASE_PAGE}
          target="_blank"
          rel="noreferrer"
          style={{
            marginTop: 4,
            padding: "12px 28px",
            borderRadius: 8,
            background: C.brand,
            color: C.onBrand,
            fontSize: 15,
            fontWeight: 500,
            textDecoration: "none",
          }}
        >
          前往下载
        </a>
        <div style={{ fontSize: 11, color: C.muted, wordBreak: "break-all" }}>{RELEASE_PAGE}</div>
      </div>
    </div>
  );
}
