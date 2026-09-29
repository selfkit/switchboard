import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import DownloadGate from "./screens/DownloadGate";

// 这份前端只在 Tauri 里有意义。普通浏览器打开的话 invoke 全会失败，
// 与其让人对着报错发呆，不如直接给下载页。
const inApp = "__TAURI_INTERNALS__" in window;

createRoot(document.getElementById("root")!).render(
  <StrictMode>{inApp ? <App /> : <DownloadGate />}</StrictMode>,
);
