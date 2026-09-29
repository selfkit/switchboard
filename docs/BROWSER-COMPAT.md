# 账号页面的浏览器行为补齐（设计）

> 目标：账号页面里该有的浏览器行为都要有（拖拽上传、点开新页签、前进后退），
> 但不把 App 变成一个浏览器外壳 —— 顶栏、侧栏、会话页那套布局和「一个账号一份隔离环境」
> 的模型不动，新能力挂在现有 `Sessions` 模型上。
>
> 相关文档：[NETWORK-GUARD.md](NETWORK-GUARD.md)（区域拦截，本文的方案必须不绕过它）。

## 0. 现状（改之前先把事实写清）

- 账号页面不是标签页，而是**盖在主窗口上的子 webview**：`session.rs` 给每个账号建一个
  label 为 `acct-<account_id>` 的 `WebviewBuilder`，一个账号**恰好一个** webview。
- 位置由 `body_rect()` 算：`x = SIDEBAR_W`、`y = TOPBAR_H(40)`，宽高吃满剩余窗口。
  前端 `AppShell` 的顶栏高度必须和 `TOPBAR_H` 对上，这是既有约定。
- 每个 webview 注入 `adapter::agent_js()`（**所有 frame**），负责填充 / 指认 / 记住最后点过的框；
  `on_page_load` 里再注入 `page_probe_js()` 让页面自报"这页能不能用"。
- 外部页面属于**远程源**，能调的命令由 `capabilities/session.json` 限定：`webviews: ["acct-*"]`
  + `remote.urls` 四个模式，权限只有 `permissions/session-report.toml` 里那两条回传命令。
- 区域拦截靠 `Sessions` 这张表逐个 webview 操作（`cut_sessions` 把页面导到 about:blank）。
  **凡是不在这张表里的 webview，守卫就管不到**——这是下面选型的硬约束。

## 1. 拖拽上传没反应

**根因**：Tauri 默认给每个 webview 装自己的拖放接收器（`drag_drop_handler_enabled = true`），
OS 的拖放事件被它接走，页面里的 `dragenter/dragover/drop` 根本不触发，于是"拖拽到此区域"
永远没反应（点击上传走的是 WKWebView 原生文件选择器，所以那条路是通的）。

**方案**：账号 webview 建的时候加 `.disable_drag_drop_handler()`。主窗口那个 App UI 的不动。

- 代价：拖文件到账号页时 Tauri 的 `DragDrop` 事件不再触发。我们从来没用过它，无损失。
- 影响面：一行。属于 P0，先上。
- 验收：腾讯云备案页把一张图拖到「点击上传/拖拽到此区域」上，出现预览。

## 2. / 3. 点开新页签没反应（`target=_blank`、`window.open`）

**根因**：wry 的 `WKUIDelegate` 只有在设置了 `new_window_req_handler` 时才实现
`createWebViewWithConfiguration:`；没设的话返回 `nil`，WKWebView 就把这次"开新窗口"的请求
**直接丢掉**。所以「公钥获取指引」「MD5值获取指引」「去续费」这类链接点了毫无反应，
既不报错也不跳转。

Tauri 2.11 把这个钩子公开成 `WebviewBuilder::on_new_window(|url, features| -> NewWindowResponse)`，
三种返回：

| 返回 | 实际行为 | 对本 App 的影响 |
|---|---|---|
| `Allow` | wry 自己开一个**原生 NSWindow**，里面一个共享 configuration 的 WKWebView | 登录态和注入脚本都在（同一份 configuration），但这个窗口**不在 `Sessions` 表里**：区域拦截切不断它、cookie 快照抄不到它、顶栏/侧栏也不在它身上 |
| `Create { window_id }` | 把该 window 的**第一个** webview 交给 WKWebView 承接 | 我们 main 窗口的第一个 webview 是 App 自己的 UI，交出去等于界面被劫持；要用就得为每个弹窗单开一个 tauri 窗口 |
| `Deny` | 丢掉请求（= 现状） | — |

**方案（选 B）**：`Deny` 掉原生的那条路，自己在**同一个账号的 profile 下**建一个新 webview 当页签：
`data_directory` 和 `data_store_identifier` 都沿用该账号 → 登录态天然共享，新页签就是"同一个浏览器里的新标签"。

选它而不是 `Allow` 的决定性理由是上表第一行最后一栏：`Allow` 开出来的窗口逃出 `Sessions`，
**区域拦截就形同虚设**，而区域拦截是这个 App 的合规底线（见 NETWORK-GUARD.md）。

**补充（已实现）**：真弹窗走 `Create`，保留 `window.opener` 语义。分流判据是
**`features.size()` 有没有值**：

- 有尺寸 = `window.open(url, name, "width=..,height=..")`，典型是第三方 OAuth 登录弹窗，
  页面还攥着返回的句柄等 `postMessage` 回来 → 开一个**独立窗口**承接，返回
  `NewWindowResponse::Create { window }`，由 WKWebView 自己接管这次 window.open。
- 没尺寸 = 多半是 `target=_blank` → 还是 App 内页签（方案 B）。

三个当初以为要自己造、其实 tauri 已经给了的点：
1. **不会死锁**：`create_window` 走 `send_user_message`，发现自己在主线程就**内联执行**
   （tauri-runtime-wry:239），所以能在 `on_new_window` 回调里同步建窗口并返回。
   只有 `create_webview` 明确要求"必须从别的线程调"——这正是 App 内页签得甩线程的原因。
2. **配置共享一行**：`WebviewWindowBuilder::window_features(features)` 把 macOS 的
   `WKWebViewConfiguration`、Windows 的 environment、Linux 的 related view 都接上，
   并套用弹窗要的尺寸位置。共享 configuration = data store 和注入脚本跟着过去，登录态不用搬。
3. **权限不用改**：弹窗 label 叫 `acct-<id>-p<n>`，capability 的 `windows` 和 `webviews`
   是**或**关系，所以它在别的窗口里也照样命中 `acct-*`（`every_tab_label_is_covered_…` 把这条钉住了）。

弹窗仍然登记在 `Sessions` 里（`Tab.external = true`），所以区域拦截照样切得断；
`refresh_layout` 跳过它（它有自己的窗框），窗口被关掉时 `Destroyed` 事件把那条页签摘掉。

剩下的小缺口：弹窗里用不了「填账号/填密码/指认」（那几个只作用于当前页签），
`window.open()` 后 `document.write` 的空白页按"没有地址的页签"处理。

### 2.1 会话模型：一个账号从"一个 webview"变成"一组页签"

```
SessionState {
    tabs: Vec<Tab>,        // Tab { label, last_url, loaded, fail, title }
    active_tab: usize,     // 页签条上高亮的那个
    platform, related_app, opened_at, cut, picking, ...   // 账号级，保持原样
}
```

- label 规则：首个页签仍是 `acct-<account_id>`（**不改**，`label_for()` 的调用方一大片，
  包括 `report_picked_selectors` 的回传匹配），后续页签 `acct-<account_id>-t<n>`，
  `n` 单调递增不复用（复用会撞上刚关掉那个 webview 的残留）。
  仍然全部命中 capability 的 `acct-*`，权限不用动。
- 下列函数从"按账号取 webview"改成"按账号取**当前页签**的 webview"：
  `fill_focused`、`set_picking` / `picking_account`、`pin_current_url`、`reload_session`、`ensure_usable`。
- 下列函数改成"遍历该账号的全部页签"：`cut_sessions`、`restore`（区域恢复）、`close_session`、
  `close_all`、`snapshot_cookies`（cookie 是 data store 级的，抄一次就够，取首个页签即可，但**必须**
  在关掉最后一个页签之前抄）。
- `SessionInfo`（给前端的）加 `tabs: Vec<TabInfo>` 和 `active_tab`；`loaded/failed/fail_reason`
  的含义改为"当前页签的"，前端现有判断不用改。

### 2.2 布局：页签条

- 只在「会话」页出现，高度常量 `TABS_H = 34`（Rust 和前端各一份，必须一致 —— 跟 `TOPBAR_H` 一样的约定，
  加一条单测钉住两边的数值）。
- `body_rect()` 改成 `y = TOPBAR_H + TABS_H`、`height -= TABS_H`。
- **固定占位**，不按"有没有第二个页签"动态收放：高度一变，webview 整块跳一下，比省掉 34px 难受。
- 视觉沿用现有色板和 `NavItem` 的选中态做法（白底 + 描边 + 轻阴影，不用"忽然变白"）：
  页签条左边是当前账号名（灰字，只读），右边是页签，超出宽度横向滚动（滚动条已全局收细）。
  每个页签：favicon 位不做，标题取 `on_document_title_changed` 的文档标题，截断 14 字，
  hover 出 ×（首个页签不给关 —— 关掉等于关账号，那件事归侧栏的 ×）。

### 2.3 "完整浏览器行为"的范围

页签条右侧补一组极小的按钮，只做这些：

| 能力 | 怎么做 | 优先级 |
|---|---|---|
| 新页签（链接 / `window.open` 不带尺寸） | `on_new_window` → Deny + 自建页签 | P1 |
| 真弹窗（`window.open` 带尺寸，OAuth） | `on_new_window` → `Create { window }` + `window_features()`，保留 `window.opener` | P4 |
| 关页签 | 关 webview，active 退回左邻 | P1 |
| 后退 / 前进 | `eval("history.back()/forward()")`（wry 没给 go_back，eval 足够） | P2 |
| 刷新 | 已有 `reload_session`，接到页签上 | P2 |
| 复制当前地址 / 在系统浏览器打开 | `last_url`；地址复制要过主密码闸门吗？**不过** —— 当前页面的地址不是账号库里的凭据。系统浏览器不加插件，直接调 OS 的 `open` / `start` / `xdg-open` | P2 |
| 下载 | `WebviewBuilder::on_download`：落设置里选的文件夹（默认系统「下载」目录），重名加 `-2`，完成后发事件告诉用户在哪；可选每次下完弹保存框另存 | P3 |
| 打印 / 右键菜单 / 缩放 | 不做（`zoom_hotkeys_enabled` 现成，真要就一行） | 不做 |

## 3. 安全与合规检查表（改完逐条对）

- [ ] 新页签的 label 命中 `acct-*`，且**只**能调那两条回传命令（新增页签不得扩权）。
- [ ] `cut_sessions` 能把一个账号的**所有**页签都切到 about:blank；恢复时也全部导回。
- [ ] 新页签的 URL 仍受 `remote.urls` 四个模式约束（含带端口的自建服务）。
- [ ] 主密码闸门不受影响：页签里的明文复制由页面自己管，我们不代劳。
- [ ] `disable_drag_drop_handler` 只关账号 webview，主窗口 UI 的拖放行为不变。

## 4. 实施顺序

1. ~~**P0 拖拽**：`.disable_drag_drop_handler()`~~ ✅ 已完成
2. ~~**P1 页签**：`SessionState.tabs` 改造 + `on_new_window` + 页签条 + 关页签~~ ✅ 已完成
   - 单测三条：`tab_labels_keep_the_first_tab_compatible`、
     `every_tab_label_is_covered_by_the_session_capability`（用 tauri 自己那套 `glob::Pattern` 对）、
     `frontend_layout_constants_match`（前后端 `TOPBAR_H` / `TABS_H` 对齐）
   - 实现时补的两件没写进设计的事：一个账号最多 `MAX_TABS = 12` 个页签
     （页签由页面 `window.open` 出来，数量不由用户控制）；退出指认模式时**所有**页签
     一起清标记 + 一起 stop，否则指认途中切过页签会留下一个永远在指认模式的页签。
3. ~~**P2 导航**：后退 / 前进 / 刷新 / 复制地址 / 系统浏览器打开~~ ✅ 已完成
   - 「系统浏览器」**过区域检测**：那边完全不受本 App 控制，出口不合规时一键跳出去
     等于给拦截开后门。只放 http(s)，别的 scheme 交给系统等于把本机当命令行使。
   - 前进后退用 `history.go(±1)`（wry 没给 go_back / go_forward）。
4. ~~**P3 下载**~~ ✅ 已完成
   - 落「设置 → 下载」里选的文件夹，没选就是系统「下载」目录（选的文件夹不在了也退回它）；重名加 `-2`…`-99`，不覆盖旧文件。
   - **「每次下载都问保存到哪」是先下后问**：文件先落进下载文件夹，下完前端弹保存框，选好后 `move_download` 挪过去，
     取消就留在原地。不在下载开始前弹框：WebKit 和 WebView2 都要在下载回调里**当场**给出落点（wry 同步回填），
     在内核回调里跑模态框等于嵌套消息循环，两个平台都不稳。跨盘另存（U 盘、网络盘）rename 会失败，退回复制再删。
   - 「在访达中显示」「另存到…」只认这次运行里本 App 下载 / 另存过的路径（`remember_download`）——路径是前端传回来的，
     不能让它拿去挪任意文件。演示模式固定存演示目录、从不弹保存框（框里会露出真实的文件夹）。
   - macOS 的 `DownloadEvent::Finished` **不带路径**（API 限制），所以自己记一份我们选的路径再报给前端。
   - 文件名来自远端，必须**先百分号解码再清洗**（`%2F` 解出来是真斜杠，顺序反了等于没清），
     且 `.` / `..` 不能当文件名 —— `dir.join("..")` 会把落点挪出「下载」目录。
     `download_names_cannot_escape_the_folder` 钉着这几条。
   - **新页签 / 弹窗里的下载**：页面 `window.open` 或 `target=_blank` 到一个下载地址时，开出来的页签第一次导航就变成了下载，
     WebKit 既不报"提交"也不报"加载完成"，页签里什么都没有，12 秒后会被加载超时判成"打不开"、整块区域变成错误提示。
     现在跟浏览器一样：下载一开始就把用户送回原来的页签，这个空页签标成「下载中…」，下载结束（成败都算）再关掉；
     空弹窗同理。不能下载一开始就关：WKDownload 的 delegate 挂在发起它的 webview 上，webview 一关就收不到「下载完成」。
     `download_only_tab_sends_user_back_and_never_fails` 钉着；演示模式控制台的「在新页签下载账单」就是这个场景。

5. ~~**P4 OAuth 弹窗**：`Create { window }` + `window_features()`，保留 `window.opener`~~ ✅ 已完成
   - `Tab.external` 标记；弹窗不参与主窗口布局，但照样受区域拦截管。
   - 关窗口（用户点窗框的 ×）和关页签（点页签上的 ×）两条路都要能收拾干净：
     前者靠 `Destroyed` 事件摘页签，后者按 `external` 关**窗口**而不是关 webview
     （只关 webview 会剩一个空窗框）。
   - 判据是启发式：不带尺寸又指望 `opener` 的站仍然会断，真撞上再按域名开白名单。

每步做完各自能验：P0 拖图上传、P1 点「MD5值获取指引」出新页签且不丢登录态、
P2 那排按钮、P3 下一个备案模板、P4 找一个第三方登录弹窗走完整条回调。
