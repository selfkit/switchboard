# Switchboard 设计与架构

本文合并两份材料：原型的屏幕清单与流程（原《原型设计说明》），以及确定内嵌 WebView 路线后的技术架构（原《架构设计文档 v2》）。

> 本文取代早期《账号管理系统-架构设计方案》里"拉起系统浏览器 + Chrome 扩展"的技术路线。

## 原型

[点击查看可交互原型](https://claude.ai/artifact/9P8Ecod9YFG9rqis2pBkde)，或本地打开 [prototype/index.html](prototype/index.html)（见 [README](../README.md#本地预览原型)）。

> 这是一份可点击跳转的 HTML 原型（非最终 UI 代码，`prototype/` 目录下的 `.dc.html` 是原型编辑器的原始格式，仅供参考排版结构，不能直接拿去当产品代码用），用来对齐产品流程和界面结构，指导后续 Tauri + 前端页面的真实实现。

## 关键架构决策

1. **不再拉起系统 Chrome，改用 Tauri 自带 WebView**：每个账号对应一个独立 `WebviewWindow`，cookie/session 存在各自 data 目录，隔离效果不变
2. **自动填充改为 App 后端直接 `eval()` 注入 JS**：不再依赖浏览器扩展 + Native Messaging
3. **Windows（WebView2/Chromium）基本零适配成本，macOS（WKWebView）是主要适配对象**：尤其是 `input.value` 赋值后要手动 `dispatchEvent` 触发页面自身校验逻辑
4. **各平台登录页选择器存成独立配置数据**（对应 `PlatformConfig.dc.html`），不硬编码在代码里，平台改版只改配置不用重新发版
5. **TOTP 是账号级可选项，不是登录必经步骤**：只有账号自己绑定了虚拟 MFA 才会出现"验证码"入口

---

# 一、原型

## 屏幕清单（21 个节点，按流程分组）

### 身份与账号库
| 屏幕 | 文件 | 作用 |
|---|---|---|
| 注册主账号 | `Register.dc.html` | 首次使用本机时，创建自己在本系统里的登录身份 |
| 登录 | `Unlock.dc.html` | 非首次使用时登录，解锁本地加密账号库 |
| 主界面 - 账号列表 | `Main.dc.html` | 登录后进入，左侧按归属/平台筛选，右侧账号卡片网格，顶部有活跃会话入口 |
| 空状态引导 | `Empty.dc.html` | 账号库为空时的首屏，引导添加第一个账号 |
| 搜索无结果 | `SearchEmpty.dc.html` | 搜索关键词没匹配到账号时的兜底，引导直接新增 |

### 账号的增删改
| 屏幕 | 文件 | 作用 |
|---|---|---|
| 新增/编辑账号 | `AddAccount.dc.html` | 登记一条外部平台账号的完整信息 |
| 字段候选/新建交互 | `FieldCandidates.dc.html` | 归属/平台/登录URL 三个字段的可搜索候选下拉演示 |
| 删除账号确认 | `DeleteConfirm.dc.html` | 二次确认，并提示"隔离 Profile 目录不会自动删除" |

### 核心业务流程 —— 一键登录（以阿里云为例）
| 屏幕 | 文件 | 作用 |
|---|---|---|
| 业务流程 - 无会话自动填充 | `EmbeddedLoginFill.dc.html` | 内嵌 WebView 打开登录页，账号密码自动填充 |
| 业务流程 - 手动指认输入框 | `EmbeddedRefill.dc.html` | 自动填充填错框时的兜底：手动点选账号框/密码框，记住这次选择 |
| 业务流程 - 会话有效直接进入 | `EmbeddedSessionValid.dc.html` | 有有效 session 时跳过登录页，直接进控制台（阿里云账号1） |
| 一键登录反馈 | `LaunchFeedback.dc.html` | 早期版本的确认态：展示对应隔离 Profile 目录和会话状态 |
| 会话过期/手动验证 | `SessionExpired.dc.html` | session 过期后需要手动确认密码 + 本地验证码的场景 |
| 本地验证码 | `TotpCode.dc.html` | 已绑定 TOTP 密钥的账号，本地生成动态验证码 |

### 多账号同时在线 / 快捷切换
| 屏幕 | 文件 | 作用 |
|---|---|---|
| 活跃会话总览 | `ActiveSessions.dc.html` | 顶部栏"N 个窗口在线"下拉面板，列出所有已打开的隔离窗口，可单独关闭 |
| 快捷切换面板（Cmd+K） | `QuickSwitch.dc.html` | 搜索框 + 分组列表：已在线的直接切换，未打开的选中即拉起新窗口 |
| 切换后 - 阿里云账号2 | `AliyunAccount2.dc.html` | 展示快切到同平台另一个账号的效果，左侧常驻账号快切栏 |
| 切换后 - 腾讯云主账号 | `TencentSwitch.dc.html` | 展示快切到另一个平台账号的效果 |

### 配置与运维
| 屏幕 | 文件 | 作用 |
|---|---|---|
| 设置页 | `Settings.dc.html` | 浏览器/WebView 相关设置、备份导出导入、修改主密码 |
| 平台适配配置 | `PlatformConfig.dc.html` | 各平台登录页的输入框选择器配置表，未配置的可去"手动指认"后自动写回 |
| 备份导出确认 | `BackupExport.dc.html` | 导出加密备份后的确认态，提醒备份与当前主密码绑定 |

## 关键流程

```
首次使用：Register → Main
非首次：  Unlock   → Main

Main → AddAccount（新增/编辑）→ Main
Main → DeleteConfirm（删除确认）→ Main
Main → Settings → PlatformConfig / BackupExport

一键登录分支（以阿里云为例）：
  点击账号
    ├─ 有有效 session → EmbeddedSessionValid（直接进入控制台）
    └─ 无 session      → EmbeddedLoginFill（自动填充登录）
                           ├─ 登录成功 → EmbeddedSessionValid
                           └─ 填错框   → EmbeddedRefill（手动指认，写回 PlatformConfig）

快捷切换：
  任意业务流程屏（左侧账号快切栏 / Cmd+K 面板）
    ├─ 已在线账号 → 直接切到 AliyunAccount2.dc.html / TencentSwitch.dc.html 等
    └─ 未打开账号 → 走一键登录分支拉起新窗口
```

---

# 二、技术架构

## 1. 总体架构

```
┌─────────────────────────────────────────────────────────┐
│                      Switchboard（桌面 App）                │
│                                                           │
│  ┌───────────────┐   invoke()   ┌──────────────────────┐ │
│  │  前端 (TS/HTML) │ ───────────▶ │   Rust 后端 (Tauri)    │ │
│  │  账号列表/设置等  │ ◀─────────── │                      │ │
│  └───────────────┘   事件回调    └───────┬──────────────┘ │
│                                          │                │
│              ┌───────────────────────────┼─────────────┐  │
│              ▼                           ▼             ▼  │
│     ┌─────────────────┐        ┌─────────────────┐  ┌────────────┐
│     │  SQLCipher 加密库 │        │  会话管理器        │  │ 平台适配配置 │
│     │  账号/凭据/配置    │        │ SessionManager   │  │ (JSON/DB)  │
│     └─────────────────┘        └────────┬─────────┘  └─────┬──────┘
│                                          │                  │
│                                          ▼                  ▼
│                              ┌───────────────────────────────┐
│                              │   N 个 WebviewWindow 实例       │
│                              │  （一个账号 = 一个隔离会话）       │
│                              │  阿里云#1 │ 阿里云#2 │ 腾讯云 …    │
│                              └───────────────────────────────┘
└─────────────────────────────────────────────────────────┘
```

分五层，职责边界很清楚：

| 层 | 职责 | 对应代码位置 |
|---|---|---|
| 表现层 | 账号列表、设置、原型里的各个 UI 屏 | `src/`（前端） |
| 应用层 | Tauri commands，前后端通信入口 | `src-tauri/src/main.rs` |
| 数据层 | 账号/凭据的加密存储 | `src-tauri/src/db.rs` |
| 会话层 | 管理"当前打开了哪些 WebviewWindow" | `src-tauri/src/session.rs`（新增，替换原 `profile.rs`） |
| 适配层 | 各平台登录页的选择器配置 | `src-tauri/src/adapter.rs`（新增）+ 数据库里的 `platform_configs` 表 |

## 2. 数据模型（在原有 Account 基础上新增）

### 2.1 账号表（沿用，见代码 `models.rs`）
不变：`id / platform / owner_type / related_app / account_role / login_url / username / credential / totp_secret / backup_contact / remark`

去掉的字段：`profile_path`（系统浏览器方案的产物，不再需要——WebView 方案里，每个账号的隔离目录由 SessionManager 按账号 id 自动生成，不需要用户可见/可编辑）

### 2.2 新增：平台适配配置表 `platform_configs`
对应原型里的 `PlatformConfig.dc.html`：

```sql
CREATE TABLE platform_configs (
    platform        TEXT PRIMARY KEY,   -- "阿里云" / "腾讯云" / "即构" / ...
    username_selector TEXT,             -- 账号输入框选择器，如 "#username"
    password_selector TEXT,             -- 密码输入框选择器
    trigger_event     TEXT,             -- "input" / "input+change" 等
    verified          INTEGER,          -- 是否验证过（0/1），对应原型里"已验证/待配置"状态
    updated_at        TEXT
);
```

选择器来源两个途径：
1. 内置默认值（针对阿里云/腾讯云主流登录页预先写好）
2. 用户走"手动指认输入框"（对应 `EmbeddedRefill.dc.html`）后，前端把选中的元素路径回传，写入这张表——这也是原型里"点一次记住，下次自动生效"的实现依据

### 2.3 会话状态（内存态，不落库）
```rust
struct SessionState {
    account_id: String,
    window_label: String,   // Tauri WebviewWindow 的 label
    opened_at: Instant,
    status: SessionStatus,  // Active / Loading / Expired
}
```
`ActiveSessions.dc.html`、`QuickSwitch.dc.html`、各业务流程屏左侧的"账号快切栏"，展示的都是这个内存态列表，App 重启后清空（重启后重新按需拉起 WebviewWindow，不需要持久化"曾经开过哪些窗口"）。

## 3. 核心模块

### 3.1 会话管理器 SessionManager（取代原 `profile.rs`）

职责：
- 维护「当前打开的 WebviewWindow」列表（内存态，见 2.3）
- 收到"一键登录"请求时：
  1. 查 `SessionState` 里这个账号是否已经开着 → 是就 `window.set_focus()` 提到前台（对应"当前已在线，直接切换"）
  2. 没开 → 创建新的 `WebviewWindow`，用账号 id 派生一个固定的 data 目录（保证同一账号每次都用同一份 cookie），跳转到 `login_url`
- 提供"关闭某个会话"、"列出所有会话"两个命令，供 `ActiveSessions.dc.html` / `QuickSwitch.dc.html` 调用

```rust
// 示意，非最终实现
#[tauri::command]
async fn switch_or_open_account(app: AppHandle, account_id: String) -> Result<(), String> {
    if let Some(existing) = SESSIONS.lock().unwrap().get(&account_id) {
        app.get_webview_window(&existing.window_label)
           .ok_or("窗口已丢失")?
           .set_focus()?;
        return Ok(());
    }
    // 没有已存在的会话，新建 WebviewWindow，data_directory 按 account_id 派生
    let login_url = db::get_login_url(&account_id)?;
    let window = WebviewWindowBuilder::new(&app, &account_id, WebviewUrl::External(login_url.parse()?))
        .data_directory(profile_dir_for(&account_id))
        .build()?;
    SESSIONS.lock().unwrap().insert(account_id.clone(), SessionState { .. });
    Ok(())
}
```

### 3.2 自动填充引擎 FillEngine

职责：账号密码"带入"到登录页输入框，对应 `EmbeddedLoginFill.dc.html` / `EmbeddedRefill.dc.html`。

流程：
1. WebviewWindow 加载完登录页后，从 `platform_configs` 取该平台的选择器配置
2. 拼一段 JS，通过 `webview.eval()` 注入执行：
   ```js
   const u = document.querySelector('{{username_selector}}');
   const p = document.querySelector('{{password_selector}}');
   if (u && p) {
     u.value = '{{username}}';
     p.value = '{{password}}';
     u.dispatchEvent(new Event('input', {bubbles: true}));
     p.dispatchEvent(new Event('input', {bubbles: true}));
     true // 返回值告诉 Rust 侧"填充成功"
   } else {
     false // 选择器没命中，触发"手动指认"兜底
   }
   ```
3. 没命中（返回 `false`，或者该平台压根没有配置）→ 前端弹出"手动指认输入框"界面，用户点选后，前端把生成的选择器（比如用户点的元素的 `id` 或最短唯一 CSS 路径）传回 Rust，写入 `platform_configs`

**平台差异要单独处理的点**（对应原型的"平台适配配置"表里"待配置/已验证"状态）：
- macOS（WKWebView）下 `dispatchEvent` 几乎是必须的，很多登录页只监听 `input` 不监听 `value` 的直接赋值
- Windows（WebView2/Chromium）下大多数情况可以更宽松，但保险起见两边都发事件
- 微信小程序这类走扫码登录的平台，`platform_configs` 里对应条目直接标"不适用"，FillEngine 跳过，交给用户在 WebView 里手动扫码

### 3.3 TOTP 模块
不变，沿用原设计（`totp.rs`），账号级可选项，跟登录流程解耦，只在账号绑了 `totp_secret` 时才在 UI 上出现入口。

### 3.4 备份/导出模块（新增，对应 `BackupExport.dc.html`）
- 导出：把当前 SQLCipher 数据库文件整个拷贝一份，文件名带时间戳，本质就是"复制一份用同一把主密码加密的文件"，不需要额外再加密一层
- 导入：选择备份文件 → 提示输入这份备份对应的主密码 → 验证通过后替换/合并当前数据库
- 换主密码后旧备份文件用不了，这个提示已经画在 `BackupExport.dc.html` 里了，代码层面只需要在"修改主密码"成功后弹一次提醒

## 4. 跨平台构建与运行

- **代码层面**：FillEngine 的注入脚本、SessionManager 的 WebviewWindow 创建逻辑，Tauri API 本身是跨平台的，不需要写 `#[cfg(target_os = ...)]` 分支，除非某个平台选择器确实需要不同的事件触发方式（届时在 `platform_configs` 里针对同一平台按 OS 存两条配置即可，不需要改代码）
- **构建层面**：`.github/workflows/build.yml` 已经配好 macOS + Windows 双平台矩阵构建，推 `v*` tag 自动出两边的安装包（`.dmg` / `.msi`）

## 5. 安全设计（沿用 + 补充）

- SQLCipher 整库加密，主密码经 Argon2 派生成数据库密钥，不落盘明文（沿用原设计）
- WebviewWindow 的 cookie/session 存在按账号 id 派生的独立 data 目录下，天然隔离，不依赖浏览器的 `--user-data-dir` 参数（原方案）而是 Tauri 的 `data_directory` API（新方案）
- 注入脚本只做"读 DOM 结构 + 填值"，不上传任何页面内容，`platform_configs` 里存的只是选择器字符串，不涉及用户在登录页里的实际输入内容
- 备份文件加密强度等于当前主密码强度，导出/导入提示已在原型里体现

## 6. 原型页面 ↔ 架构模块对照表

| 原型屏 | 对应后端模块 |
|---|---|
| `Main.dc.html` | 数据层（账号列表查询）+ 会话层（顶部"N个窗口在线"读 SessionManager） |
| `AddAccount.dc.html` / `DeleteConfirm.dc.html` | 数据层（增删改） |
| `EmbeddedLoginFill.dc.html` | 会话层（新建 WebviewWindow）+ 适配层（FillEngine 首次尝试） |
| `EmbeddedRefill.dc.html` | 适配层（写回 `platform_configs`） |
| `EmbeddedSessionValid.dc.html` | 会话层（复用已有 WebviewWindow） |
| `ActiveSessions.dc.html` / `QuickSwitch.dc.html` | 会话层（SessionManager 状态查询/切换） |
| `AliyunAccount2.dc.html` / `TencentSwitch.dc.html` | 会话层（`switch_or_open_account` 命令） |
| `TotpCode.dc.html` | TOTP 模块 |
| `PlatformConfig.dc.html` | 适配层（`platform_configs` 表的可视化管理） |
| `BackupExport.dc.html` / `Settings.dc.html` | 备份/导出模块 |

## 7. 与代码脚手架的差距（待实现清单）

当前 `account-manager.zip` 里的代码还停留在"拉起系统浏览器"的旧方案，按本文档要改的地方：

- [ ] 删除 `profile.rs`，新增 `session.rs`（SessionManager）、`adapter.rs`（FillEngine）
- [ ] `models.rs` 去掉 `profile_path` 字段，新增 `PlatformConfig` 结构体
- [ ] `db.rs` 建表语句加上 `platform_configs`
- [ ] `main.rs` 新增命令：`switch_or_open_account`、`list_active_sessions`、`close_session`、`save_platform_selector`、`export_backup`、`import_backup`
- [ ] 前端补齐"活跃会话面板""快捷切换面板"两个新界面（原型已有，代码还没写）

## 8. 尚未覆盖 / 可以继续深入的方向

- 多设备同步（当前设计只有本地加密备份的导出/导入，没有云端同步）
- 移动端适配（目前原型全部按桌面 1280×800 设计）
- 平台改版导致选择器失效时的主动告警机制

---

# 三、实现与本文的差异（代码为准）

本文第二部分是**动工前的设计**，实现过程中有几处按现实改掉了。以下以代码为准：

| 本文原先写的 | 实际实现 | 为什么改 |
|---|---|---|
| 每个账号一个独立 `WebviewWindow` | 全部账号页面是**主窗口的子 webview**，左边账号栏、右边页面，同时只 show 一个 | 窗口开多了没法用。隔离没打折：每个子 webview 仍各自一份 cookie 存储 |
| 靠 `data_directory` 做隔离 | macOS 走 `data_store_identifier`（由账号 uuid 派生），Windows/Linux 才走 `data_directory`，两个都设 | **WKWebView 完全无视 `data_directory`**，macOS 上只认 `WKWebsiteDataStore` 的 identifier（需 macOS 14+） |
| 主密码经 Argon2 派生成库密钥 | 主密码直接作为 SQLCipher 的 key | SQLCipher 内部已是 PBKDF2-HMAC-SHA512 256k 轮；"验证密码"顺带免费（开库后第一条查询失败即密码错） |
| `eval()` 返回值告诉 Rust 填充成功与否 | `eval` 是单向的，拿不到返回值。改为：填充失败不报错，由用户走「填账号/填密码」或「指认输入框」兜底 | Tauri 的 `eval` 没有返回通道 |
| 备份 = 拷贝数据库文件 | `VACUUM INTO` | 不用停机，产出的是一致快照，且仍用同一把主密码加密 |
| （未设计） | **会话 cookie 快照**：退出/锁库/关会话时把 cookie 抄进加密库，下次打开塞回去 | 登录态多是会话 cookie，WebView 一关就没，否则每次开 App 都要重登 |
| （未设计） | **在线适配**：平台选择器可用 JSON 导入，配套提示词让 AI 生成 | 不可能为每个新网站重新打包 |
| （未设计） | **强制重置**：`switchboard reset`，且必须在数据目录里执行 | 主密码即密钥，忘了只能丢库重来；这一步不该是误点能触发的 |

还有一条实现上的硬约束：**wry 在 macOS 上没有实现 WKWebView 的 JS 弹窗代理**，所以 `window.alert / confirm / prompt` 在 App 里静默失效，所有确认/输入都必须自己画。
