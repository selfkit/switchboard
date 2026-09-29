import type { Account } from "../api";
import { Button, C } from "../ui";

/** DeleteConfirm.dc.html —— 叠在 Main 上方的弹层 */
export default function DeleteConfirm({ account, onCancel, onConfirm }: { account: Account; onCancel: () => void; onConfirm: () => void }) {
  return (
    <div
      onClick={onCancel}
      style={{ position: "fixed", inset: 0, background: "rgba(0,0,0,0.35)", display: "flex", alignItems: "center", justifyContent: "center" }}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{ width: 380, background: C.white, borderRadius: 14, padding: 28, display: "flex", flexDirection: "column", gap: 16 }}
      >
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <div style={{ fontSize: 18, color: C.text }}>删除这个账号？</div>
          <div style={{ fontSize: 13, color: C.sub, lineHeight: 1.6 }}>
            「{account.platform} · {account.related_app || account.username}」将从账号库移除，加密记录会被清除
          </div>
        </div>

        <div style={{ background: C.bg, borderRadius: 8, padding: "12px 14px", fontSize: 12, color: C.sub, lineHeight: 1.6 }}>
          这个账号的登录态（cookie 快照）会一并删掉；WebView 在系统里留下的缓存目录不会自动清理，需要的话去数据目录手动删
        </div>

        <div style={{ display: "flex", gap: 10, justifyContent: "flex-end" }}>
          <Button onClick={onCancel} style={{ padding: "9px 16px", fontSize: 13 }}>
            取消
          </Button>
          <Button kind="danger" onClick={onConfirm} style={{ padding: "9px 16px", fontSize: 13 }}>
            确认删除
          </Button>
        </div>
      </div>
    </div>
  );
}
