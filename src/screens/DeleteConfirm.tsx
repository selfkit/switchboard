import type { Account } from "../api";
import { Button, C, Dialog, R } from "../ui";

/** DeleteConfirm.dc.html —— 叠在 Main 上方的弹层 */
export default function DeleteConfirm({ account, onCancel, onConfirm }: { account: Account; onCancel: () => void; onConfirm: () => void }) {
  return (
    <Dialog label="删除账号" width={400} onClose={onCancel}>
      <div style={{ padding: 28, display: "flex", flexDirection: "column", gap: 16 }}>
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <div style={{ fontSize: 18, color: C.text, fontWeight: 500 }}>删除这个账号？</div>
          <div style={{ fontSize: 13, color: C.sub, lineHeight: 1.6 }}>
            「{account.platform} · {account.related_app || account.username}」将从账号库移除，加密记录会被清除
          </div>
        </div>

        <div style={{ background: C.bg, borderRadius: R.md, padding: "12px 14px", fontSize: 12, color: C.sub, lineHeight: 1.6 }}>
          这个账号的登录态（cookie 快照）会一并删掉；WebView 在系统里留下的缓存目录不会自动清理，需要的话去数据目录手动删
        </div>

        <div style={{ display: "flex", gap: 10, justifyContent: "flex-end" }}>
          <Button size="md" onClick={onCancel}>取消</Button>
          <Button size="md" kind="danger" onClick={onConfirm}>确认删除</Button>
        </div>
      </div>
    </Dialog>
  );
}
