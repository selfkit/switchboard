import { useEffect, useRef, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import * as api from "../api";
import { C, FILE_MANAGER } from "../ui";

/** App 根层常驻；后端保留未确认事件，切页/重载/锁定都不会丢另存请求。 */
export default function DownloadCenter({ unlocked }: { unlocked: boolean }) {
  const allowed = useRef(unlocked);
  allowed.current = unlocked;
  const busy = useRef(false);
  const [recent, setRecent] = useState<{ id: number; name: string; path: string; error?: string }[]>([]);
  const [expanded, setExpanded] = useState(false);
  useEffect(() => {
    let live = true;
    const drain = async () => {
      if (!allowed.current || busy.current || !live) return;
      busy.current = true;
      try {
        const pending = await api.pendingDownloads();
        for (const d of pending) {
          if (!live || !allowed.current) break;
          let path = d.path, error = d.success ? "" : "下载失败";
          if (d.success && d.ask) {
            const to = await save({ title: `保存「${d.name}」`, defaultPath: d.path }).catch(() => null);
            if (to) {
              try { path = await api.moveDownload(d.path, to); }
              catch (e) { error = `另存失败，原文件仍在下载目录：${String(e)}`; }
            }
          }
          if (!live) break;
          setRecent(items => [{ id: d.id, name: path.split(/[\\/]/).pop() || d.name, path, error }, ...items.filter(x => x.id !== d.id)].slice(0, 20));
          await api.acknowledgeDownload(d.id);
        }
      } catch (_) { /* 锁库期间继续保留事件，解锁后再取。 */ }
      finally { busy.current = false; }
    };
    const un = api.onDownloadFinished(() => { void drain(); });
    const timer = window.setInterval(() => { void drain(); }, 2000);
    void drain();
    return () => { live = false; window.clearInterval(timer); void un.then(f => f()); };
  }, []);
  if (!unlocked || recent.length === 0) return null;
  const shown = expanded ? recent : recent.slice(0, 1);
  return (
    <aside aria-label="下载记录" style={{ position: "fixed", left: 8, bottom: 72, width: 208, maxHeight: "50vh", overflowY: "auto", zIndex: 50, background: C.surface, border: `1px solid ${C.border}`, borderRadius: "var(--sb-radius-md)", padding: 8, boxSizing: "border-box", fontSize: 12 }}>
      <button onClick={() => setExpanded(v => !v)} style={{ color: C.brand, cursor: "pointer" }}>下载记录（{recent.length}）{expanded ? "收起" : "展开"}</button>
      {shown.map(d => <div key={d.id} style={{ marginTop: 6 }}>
        <button title={d.path} disabled={!!d.error && !d.path} onClick={() => { void api.revealDownload(d.path).catch(e => setRecent(items => items.map(x => x.id === d.id ? { ...x, error: String(e) } : x))); }} style={{ color: C.brand, cursor: "pointer", wordBreak: "break-all" }}>{d.name || "文件"} · 在{FILE_MANAGER}中显示</button>
        {d.error && <div role="alert" style={{ color: C.danger }}>{d.error}</div>}
      </div>)}
      <button onClick={() => setRecent([])} style={{ marginTop: 6, cursor: "pointer" }}>清除记录</button>
    </aside>
  );
}
