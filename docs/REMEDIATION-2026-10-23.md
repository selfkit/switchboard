# 2026-10-23 审计问题修复

依据：[2026-10-22 审计报告](/Users/macbookpro/Desktop/tan/code/me/switchboard/docs/AUDIT-2026-10-22.md)。本轮已为 A01～A11 实现代码修复，并升级可兼容更新的安全依赖。下表区分自动验证结果与仍需原生实机验收的部分；不将交叉编译视作 Windows 功能验收。

## 修复记录

| 问题 | 本轮变更 | 验证及边界 |
|---|---|---|
| A01 凭据跨来源 | Rust 与两个网页代理共用明确的来源策略；默认要求同源，协议和端口均纳入比较。已知阿里云登录来源逐个列出。执行前再次检查实际页面来源 | 回归覆盖不同 github.io 租户、未授权兄弟子域、异常端口、HTTPS 降级，以及已授权 iframe 正常填写 |
| A02 旧版指认目标错误 | 指认绑定页签、来源、随机 nonce 和 60 秒期限；导航、切页签、锁定使许可失效；拒绝非活动账号回调。回调只读取平台/登录地址并保存选择器，不再读取密码或触发立即填写 | 许可的 nonce、来源和过期检查有 Rust 测试；原生导航和迟到 IPC 组合仍需实机验收 |
| A03 锁定后创建窗口 | 新窗口统一检查解锁代次、账号会话代次和区域切断状态；创建中的窗口也计入数量上限。弹窗先隐藏，子 WebView 先在屏幕外创建并隐藏，再导航和登记；完成时重新验证，过期创建销毁 | macOS 与 Windows 分支均编译通过；原生锁定与开窗竞争仍需实机验收 |
| A04 伪造活动心跳 | 移除远端 touch_activity 权限和网页心跳转发。macOS 使用 AppKit 本地输入，Windows 读取系统输入变化并要求 App 位于前台；本地主窗口保留活动命令 | 网页消息不能续期、远端 ACL 不再允许该命令；实际网页焦点下的输入计时需实机验收 |
| A05 HTTP 字段绑定 | 缺少 crypto.randomUUID 时使用 crypto.getRandomValues 生成随机文档标识 | HTTP 局域网站点模型中执行实际代理，定位和填写通过 |
| A06 修改密码框误填 | 自动填写排除 new-password（含 section token）、非空、只读、禁用和错误类型的控件；赋值前再次检查。仅允许登录直达页的同一路径，SPA 路由变化结束旧轮询 | 旧 agent 回归覆盖新密码框、已输入内容、只读与禁用控件；复杂 SSO 路由可能需更新登录直达地址或使用字段绑定/主动填写 |
| A07 备份预检不足 | 在临时加密副本上检查完整性，完成 schema 迁移，并读取账号、适配与绑定；检查失败即清理副本，成功后才锁定和替换当前库。原备份保持原样 | 不完整 accounts 表被拒绝；旧 schema 备份在副本上迁移成功；原备份字节不变；失败不留临时数据库 |
| A08 并发下载路径错配 | 每个下载按 URL 关联路径，优先使用原生完成路径。缺少原生下载 ID 时，同页签相同 URL 的并发请求明确拒绝，不覆盖已有映射；在途路径也参与文件名避让 | A/B 交错完成、原生路径优先、重复 URL 不覆盖映射有 Rust 回归；原生下载回调仍需实机验收 |
| A09 切页面丢下载事件 | 后端保留未确认的完成记录；App 根层常驻下载中心，串行处理另存提示，锁定时暂停、解锁后继续。提供本次界面运行期间最近 20 条处理记录 | TypeScript 与生产混淆构建通过；需实机验证系统保存框、锁定期间完成和页面切换组合 |
| A10 SPA 地址陈旧 | 从原生 WebView 同步当前 URL，查询时不持有会话锁；列表、刷新和切断前同步实际路由。macOS 通过可空原生 URL 查询避免未提交导航的 nil URL 问题；保活也复用此查询 | 两平台编译通过；history/hash 路由与区域恢复的原生组合需实机验收 |
| A11 网页焦点快捷键 | 增加应用原生菜单 Cmd/Ctrl+K，从原生事件打开切换面板；保留主窗口入口，避免覆盖复制确认和区域告警 | 两平台编译通过；网页输入框、iframe、弹窗焦点需实机验收 |

主要实现：[来源策略](/Users/macbookpro/Desktop/tan/code/me/switchboard/src-tauri/src/origin_policy.rs)、[明确来源列表](/Users/macbookpro/Desktop/tan/code/me/switchboard/src-tauri/src/credential-origins.json)、[会话与窗口](/Users/macbookpro/Desktop/tan/code/me/switchboard/src-tauri/src/session.rs)、[原生活动来源](/Users/macbookpro/Desktop/tan/code/me/switchboard/src-tauri/src/activity.rs)、[下载关联与队列](/Users/macbookpro/Desktop/tan/code/me/switchboard/src-tauri/src/downloads.rs)、[全局下载中心](/Users/macbookpro/Desktop/tan/code/me/switchboard/src/screens/DownloadCenter.tsx)。

## 兼容性变化

- 自定义站点的兄弟子域、不同端口和 HTTP 降级不再被自动视为可信来源。确实需要其他登录站点时应配置准确的登录直达 URL；扩展 SSO 必须逐个审查和加入明确列表，不能恢复按域名末尾猜测的规则。
- 旧版指认完成后只保存配置，下一次打开/刷新或主动填写时才交付凭据。
- 旧版自动填写要求登录直达页同一路径，并默认不覆盖已有输入。重定向到其他路径时应更新直达 URL，或使用页面条件明确的字段绑定/主动填写。
- 同页签同 URL 文件下载必须等待前一个完成；不同 URL 可以并发。未确认下载完成记录保存在后端进程内，可跨前端重载，退出整个 App 后不保留。
- 自动锁定不再相信网页发送的心跳；Windows 使用系统输入时间与前台窗口联合判断。其他非 macOS/Windows 平台没有新增网页原生输入监听，需要另行适配。

## 依赖更新

| 包 | 原版本 | 修复后锁定版本 |
|---|---|---|
| vite | 5.4.21 | 6.4.5 |
| esbuild | 0.21.5 | 0.25.12 |
| source-map-js | 1.2.1 | 1.2.2 |
| rustls | 0.23.45 | 0.23.46 |
| rustls-webpki | 0.103.15 | 0.103.16 |
| quinn-proto | 0.11.17 | 0.11.19 |
| webpki-root-certs | 1.0.9 | 1.0.10 |

`pnpm audit --json` 复查为 **0 条告警**。对更新后的 Cargo.lock 再次批量查询 OSV：556 个 registry 包版本中，仍有 11 个包版本命中公告。结果保存在[复查清单](/Users/macbookpro/Desktop/tan/code/me/switchboard/docs/REMEDIATION-2026-10-23-dependencies.json)。

剩余 Rust 命中包括 9 个停止维护/迁移提示包，以及 glib 和 zip。glib 属于 Linux GTK 分支；zip 4.6.1 由 Windows 更新器引入，本次命中公告的关键条件涉及 Unix 提取权限/符号链接。没有将这些告警标记为已修复，也没有直接跨主版本替换上游依赖。它们的适用性分析沿用原审计报告，后续仍需随 Tauri/updater 的兼容升级处理。

## 验证

- `pnpm check:agent`：通过；包含跨租户与未授权兄弟来源、真实值投递、伪造活动、新密码/已有输入保护。
- `pnpm check:bindings`：16 项通过；包含 HTTP 缺少 randomUUID、明确授权的跨源 iframe、原有路径/取消/迟到回执检查。
- `pnpm build:app`：TypeScript、Vite 6.4.5 生产构建、混淆步骤通过。
- `cargo test --offline --manifest-path src-tauri/Cargo.toml`：84 项通过，1 项联网探针保持忽略。
- `cargo check --offline --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-gnu`：通过。
- `git diff --check`：通过。

完整 Rust 测试在允许本地服务与 macOS CoreImage 的正常权限下运行。测试数据库与账号均为虚构临时数据。

## 发布前原生验收

1. 用测试账号开启保活，设置定时 window.open，再分别手动锁定、自动锁定和系统锁屏；解锁前不得出现账号页或独立弹窗。并发开窗中锁定、关闭并重新打开账号，也不得登记旧请求。
2. 指认中导航、切页签/账号、锁定、等待超过 60 秒；旧回调不得保存配置，更不能触发填写。
3. 在账号页输入框、iframe 和独立弹窗中连续操作，确认空闲计时正常；仅让网页定时发送消息时仍应自动锁定。
4. 同页 A/B 下载交错完成、重复 URL 下载、切账号库/设置页、下载完成前锁定；确认路径准确、提示串行且解锁后可处理。
5. SPA 使用 pushState/replaceState/hash 切路由，检查地址显示、复制、普通刷新、区域切断恢复均停留在当前路由。
6. macOS 与 Windows 分别检查网页焦点下 Cmd/Ctrl+K、原生保存框及授权 SSO 登录；不能用当前编译结果替代这轮验收。

原报告中独立恢复向导、备份合并、选择器主动告警、更多 TOTP 格式、多设备同步和移动端等能力仍属于后续产品工作。本轮优先完成已知代码缺陷和兼容的依赖安全更新。
