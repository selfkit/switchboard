# 加密与数据安全

账号库里放的是各云平台的密码、TOTP 密钥和登录状态，这份文档讲清楚它们在哪、怎么加密、什么时候是明文、
锁定 / 备份 / 恢复时各发生了什么，以及这套设计**挡得住什么、挡不住什么**。改动相关代码前先读一遍末尾的「红线」。

## 1. 防什么，不防什么

| 场景 | 结论 |
|---|---|
| 电脑丢了 / 库文件、备份文件被拷走 | **挡得住**，前提是主密码够强（见 §9） |
| 别人坐到你电脑前，App 是锁着的 | 挡得住：锁定后磁盘上只剩加密库，登录态也清掉了（开了保活的账号除外，见 §5） |
| 别人坐到你电脑前，App 开着没锁 | 挡不住，只能靠自动锁定缩短窗口（默认 30 分钟） |
| 本机有以你身份运行的恶意程序，App 解锁中 | 挡不住：密钥和正在用的明文都在内存里。任何本地密码管理器都一样 |
| 外部登录页里的脚本想读账号库 | 挡得住：账号页面只能调 3 条回传命令（见 `permissions/session-report.toml`） |
| 往 App 自己的界面里注入脚本 | CSP 兜底（见 §4.4），界面本身也没有插入原始 HTML 的地方 |

## 2. 数据都在哪

| 位置 | 内容 | 加密 | 什么时候存在 |
|---|---|---|---|
| `<数据目录>/switchboard.db` | 账号、密码、TOTP 密钥、附加字段、cookie 快照、平台配置、各项设置 | SQLCipher 整库加密 | 一直 |
| `<数据目录>/owner.txt` | 注册时填的用户名（登录页显示"当前账号是谁"） | **明文** | 一直，只有名字，不含任何凭据 |
| `<数据目录>/switchboard.db.replaced-*` | 导入备份前的旧库 | 用**当时**的主密码加密 | 每种只留最近 3 份 |
| `<数据目录>/switchboard.db.locked-*` | 清空账号库时归档的旧库 | 用**当时**的主密码加密 | 每种只留最近 3 份 |
| `~/Library/WebKit/<bundle id>/WebsiteDataStore/<账号 id>`（Windows：`<数据目录>\profiles\<账号 id>`） | 各账号页面的 cookie、LocalStorage、IndexedDB | **明文**（WebKit 自己存的） | 只在解锁期间；锁定、删除账号、清空时抹掉（保活账号锁定时不抹） |
| 导出的 `switchboard-backup-*.db` | 除登录态以外的整库 | 同一把主密码加密 | 用户自己保管 |
| 系统剪贴板 | 复制 / 填充时放进去的明文 | 明文 | 30 秒后自动清掉 |

`<数据目录>`：Windows 是 `%APPDATA%\com.switchboard.app\`；macOS 是 `~/Library/Application Support/com.switchboard.app/`（开发版在其下的 `dev/`，演示模式再下一层 `demo/`）。
开发版没打包成 .app，WebKit 那一级的 `<bundle id>` 是可执行文件名 `switchboard`。

## 3. 账号库加密

用的是 SQLCipher，没有自己写任何加密算法。下面这些参数是对程序里实际链接的库执行 `PRAGMA cipher_version` / `kdf_iter` 等拿到的，不是照文档抄的：

| 项目 | 值 |
|---|---|
| 库 | SQLCipher 4.5.7 community，底层 OpenSSL 3.6.x（rusqlite `bundled-sqlcipher-vendored-openssl`，随 App 一起编译） |
| 加密单位 | 整个文件按 4096 字节一页逐页 AES-256 加密 |
| 完整性 | 每页 HMAC-SHA512，文件被改过就打不开 |
| 密钥派生 | 主密码 → PBKDF2-HMAC-SHA512，256,000 轮 |
| 盐 | 每个库文件开头 16 字节随机盐，同一个密码建两个库，密钥也不一样 |
| 回滚日志 | `journal_mode=delete`，日志文件同样加密 |
| 临时表 | 编译时 `SQLITE_TEMP_STORE=2`，只在内存里，不落盘 |

几个关键点：

- **主密码哪儿都不存，连哈希都不存。** 验证方式就是拿它去解密：`db::open_for_unlock` 执行 `PRAGMA key` 后读一次 `sqlite_master`，
  读得出来就对，读不出来就是"主密码不正确"。
- **空密码在后端就挡掉。** SQLCipher 收到空 key 会建出一个**根本没加密**的库，只靠前端校验挡不住（`register` / `MIN_PASSWORD_LEN`）。
- 库不存在时不拿去验：SQLite 打开不存在的文件会就地造一个空库，任何密码都"验证通过"（`unlock` / `import_backup` 开头的判断）。

## 4. 解锁之后

### 4.1 解锁流程

1. 登录页把主密码经 Tauri 的进程内 IPC 交给后端（不走网络）。
2. `unlock` 用它打开库，先读出区域检测设置，**检测通过之前不把连接交给界面**。
3. 通过后执行 `db::prepare`（建表、老库升级、补默认平台配置），连接放进 `Vault`，整个解锁期间保持打开。

解锁期间，派生出来的密钥在后端进程内存里。`cipher_memory_security` 是 SQLCipher 的默认值 0（不锁页、不擦释放的内存）。

### 4.2 界面不常驻明文

`list_accounts` 下发给界面的账号是**打过码**的：密码、TOTP 密钥、标了"敏感"的附加字段，只要有值就换成 `••••••`（`lib.rs` 的 `masked`），
界面只能拿它判断"有没有"。真要用明文时单独取，用完就丢：

| 操作 | 怎么拿明文 |
|---|---|
| 编辑、复制新建 | `get_account` 取完整的一条再进编辑页 |
| 复制密码 / 复制整条账号 | 验完主密码才调 `get_account` |
| 填账号 / 填密码 / 填验证码 / 填附加字段 | `fill_focused` 在后端取值、注入页面，顺带把值返回给界面放进剪贴板 |
| 看验证码 | `totp_code` 在后端算好只返回 6 位数字 |

兜底：`save_account` 会把打码的值换回库里的原值（`unmask`）。哪条路径漏了、把打过码的账号拿去存，也不会把真密码覆盖成一串圆点；
新账号带着打码的值直接拒。

### 4.3 复制与剪贴板

- 复制账号信息、密码、验证码要先验一次主密码（另开一个连接试解密，验过后 5 分钟内不再问，锁定即失效）。
  每次试解密都要跑一遍 256,000 轮，本身就慢，但没有"输错几次锁住"的限制。
- 所有明文进剪贴板后 30 秒自动清掉——前提是剪贴板里还是那一段，用户这期间复制了别的就不动（`clear_clipboard_later`，
  用 `pbpaste` 比对；GUI 进程没有 `LANG`，必须显式指定 UTF-8，否则中文密码永远比不上）。

### 4.4 CSP

主窗口配了内容安全策略（`tauri.conf.json` → `app.security.csp`）：只允许加载 App 自己的脚本，禁止 `eval` 和内联脚本，
网络只能走 Tauri 的 IPC（`ipc:` / `http://ipc.localhost`），不能嵌 iframe、不能提交表单到外面。
界面同时握着后端全部命令的权限，万一被注入脚本就等于全部泄露，CSP 是这一层的兜底。

- 样式放行 `'unsafe-inline'`：`index.html` 里有一段内联 `<style>`，React 的 `style` 属性走 CSSOM 不受限制。
- `dangerousDisableAssetCspModification: true`：不让 Tauri 往 `style-src` 里塞哈希——塞了之后浏览器会忽略 `'unsafe-inline'`。
  界面 HTML 里没有内联脚本，脚本那边本来也用不上 Tauri 的 nonce。
- 开发版单独一条 `devCsp`：Vite 热更新要内联脚本和 `ws://localhost:1420`。
- 只作用于 App 自己的界面，账号页面（外部登录页）不受它管，那边靠权限表隔离。

改完 CSP 的验证办法：`vite build` 后用带同样 `Content-Security-Policy` 头的静态服务器打开 `dist/`，控制台不能有违规报错，
页面要能正常渲染；再确认注入 `eval('1')` 和内联 `<script>` 都被拦下。

### 4.5 自动填充

密码会嵌进注入登录页的那段脚本里（`adapter::fill_js`）。脚本是页面每次加载完时现从加密库里取的（`autofill_for`），
不在开页面时拼好攥着：改了密码或选择器刷新就生效，明文也不会在页面整个生命周期里一直留在内存里。页面里再往下分发时只定向发给跟主文档同站的那一帧，
不会广播给页面上的广告 / 统计 iframe（详见 `adapter.rs` 顶部说明和 `pnpm check:agent`）。
只在跟登录直达 URL 同站的页面、且打开或刷新后 2 分钟内才填。

## 5. 锁定

手动锁定、自动锁定、清空账号库、导入备份都走同一个收尾 `lock_down`：

1. 把各账号页面当前的 cookie 抄进加密库（`snapshot_cookies`）。
2. 抹掉 WebKit 留在磁盘上的登录数据：开着的页面先让 WebKit 自己清空 data store 再关，关着的直接删目录（`session::purge`）。
3. 关掉所有账号页面。
4. 放开数据库连接（密钥随之释放），界面清空账号列表。

所以锁上之后，磁盘上只剩加密库；下次打开账号时由加密库里的 cookie 快照把登录态接回来。

**例外：开了保活的账号**（会话页左栏勾的，按账号存在 meta 的 `keep_alive_accounts`）。手动锁定、自动锁定时它们的页面不关、
登录数据不抹，藏在锁屏后面照样定时刷新（`lock_down` 的 `keep_alive` 参数）。库照样锁上、密钥照样放掉，界面上看不到这些页面
（`set_sessions_visible` 锁着时一律不显示，独立弹窗直接关掉）；区域哨兵锁着时照旧管它们，用锁定前读到的检测设置。
清空账号库、导入备份、演示模式不留；从锁屏进演示前先把它们抹掉。代价：这几个账号的登录数据锁定期间明文留在磁盘上，
锁着时 cookie 快照写不进库，⌘Q 退出会丢掉锁定之后才变的会话 cookie。

- **代价**：登录凭证放在 LocalStorage 里的站（不少自建后台），锁一次要重新登一次。阿里云、腾讯云这类 cookie 登录的站不受影响。
- **自动锁定默认 30 分钟**（从没设过的库也按 30 分钟；用户明确选过「关闭」的照旧关着）。演示库固定关闭。
- **⌘Q 退出不经过锁定**（macOS 的 `terminate:` 不触发 `ExitRequested`），磁盘上的登录数据会留到下次锁定时才抹。
  cookie 快照另有每 120 秒一次的定时兜底。

## 6. 备份导出

`export_backup` → `db::export_backup`：

1. `VACUUM INTO` 出一份一致的副本，用**主库同一把钥匙**加密。不需要停机拷文件，也不再额外套一层加密。
2. `ATTACH` 这份副本（不带 KEY 时 SQLCipher 沿用主库钥匙），删掉 `session_cookies`，再 `VACUUM` 一遍，被删的数据不留在空闲页里。

结果：备份里有账号、密码、TOTP 密钥、平台配置和设置，**没有登录态**。备份文件会被拷到 U 盘、网盘、别的电脑，
带着 cookie 快照等于随手散发一堆免密码、免两步验证的登录凭证。恢复后各账号重新登录一次就行。

一份备份安不安全，完全取决于**导出那一刻的主密码**。之后改了主密码，旧备份仍然只能用旧密码打开，也仍然只受旧密码保护。

## 7. 恢复导入

`import_backup` 要两个密码：

- **这份备份当时的主密码**：只读地打开验证（`open_for_unlock` + `count_accounts`），不会往用户选的文件里写任何东西。
- **当前账号库的主密码**：整库替换跟「清空账号库」同一级别，不验的话拿一份自己知道密码的备份导进来就绕过了那道门槛。

验证通过后：走 `lock_down` → 当前库改名成 `switchboard.db.replaced-<时间>`（重名自动加 `-2`、`-3`，不会盖掉上一份）→
复制备份进来 → 清理多余的旧归档。之后账号库的主密码就是这份备份的密码，App 锁回登录页。复制失败会把原库挪回去。

## 8. 改主密码、清空账号库、删除账号

- **改主密码**（`change_master_password`）：先另开连接验当前密码，再 `PRAGMA rekey` 用新钥匙把整个库重新加密一遍，旧密码立刻失效。
  旧备份和旧归档不受影响，仍是旧密码。
- **清空账号库**（`wipe_vault`）：要主密码。走 `lock_down`（抹掉所有账号的登录数据），删掉 `profiles/`，库改名成
  `switchboard.db.locked-<时间戳>`（仍是旧密码加密，不是直接删除），清理多余的旧归档，删掉 `owner.txt`。
- **删除账号**：连同它在磁盘上的登录数据一起抹掉（`session::purge`），库里的账号和它的 cookie 快照一起删。
- **清除登录状态**（会话页，`clear_login`）：只动登录态不动账号——关掉页面、抹掉磁盘上的登录数据、删掉加密库里的 cookie 快照，
  下次打开需要重新登录。给"在网站上退出登录后，旧快照又把人自动登回去"兜底：退出后 cookie 可能一条不剩，
  而快照遇到空值会跳过（免得读取失败时误删有效登录），旧快照就一直留着。
- 旧归档每种只留最近 3 份（`prune_archives`）：它们用的是当时的主密码，那把可能比现在的弱，一直堆着等于给离线破解多留靶子。
- 命令行 `switchboard reset`（忘记主密码时用）同样只是改名归档，不删文件。

## 9. 已知局限与取舍

1. **整套加密的强度等于主密码的强度。** PBKDF2 跑 256,000 轮能拖慢暴力破解，但它不像 Argon2 那样吃内存，可以用显卡批量猜。
   随机且足够长的主密码离线破解不可行；短密码、常见密码，被拷走的库或备份可以在离线环境里试出来。
   最短长度是 3 位（`MIN_PASSWORD_LEN`），这是有意保留的，强度由用户自己把握——**请用长一点的主密码**。
2. **解锁期间明文在内存里**：后端有密钥，界面在用户操作时会短暂拿到明文。缓解手段是自动锁定和界面打码，挡不住本机恶意程序。
3. **TOTP 密钥和密码在同一个库里**：谁拿到库加主密码，两步验证就形同虚设。特别敏感的账号可以不存密钥，继续只用手机。
4. **⌘Q 退出时 WebKit 的登录数据留到下次锁定**，见 §5。
5. **owner.txt 是明文用户名**，只是为了登录页显示身份。
6. **Windows 版还没在真机上完整跑过**（见 RELEASE.md「Windows 版」）。那边的登录数据在数据目录下的 `profiles\<账号 id>`，
   同样会被抹掉；WebView2 关掉后会占着文件几秒，删不掉时后台重试（`session::retry_remove`），账号又被打开就停手。
   剪贴板自动清空在 Windows 上用隐藏窗口的 PowerShell 比对后清空，值走环境变量不进命令行。Linux 没有做适配。

## 10. 测试钉住的行为

| 测试 | 钉住什么 |
|---|---|
| `db::tests::database_on_disk_is_actually_encrypted` | 文件开头不是明文 SQLite 魔数，账号名在文件里搜不到，空密码被拒 |
| `db::tests::roundtrip_and_reject_wrong_password` | 对的密码读得出，错的读不出 |
| `db::tests::unlock_precheck_does_not_write_before_prepare` | 解锁前的预检不写库 |
| `db::tests::backup_copy_keeps_the_same_key` | `VACUUM INTO` 的副本只认同一把密码 |
| `db::tests::exported_backup_drops_session_cookies` | 导出的备份不带登录态，当前库自己的快照不受影响 |
| `db::tests::rekey_switches_the_master_password` | 改主密码后旧密码失效、数据还在 |
| `tests::masked_accounts_never_overwrite_real_secrets` | 列表打码；存回时换回原值；新账号带打码值被拒 |
| `tests::only_the_newest_archives_are_kept` | 旧归档只留最近 3 份，别的文件一个不碰 |
| `session::tests::store_dir_matches_the_account_uuid` | 抹登录数据时找的目录名就是账号 id；带 `..` 的 id 不拿去删 |
| `tests::every_command_is_covered_by_a_permission` | 命令表和权限表同步 |

跑法：`cargo test --manifest-path src-tauri/Cargo.toml`。

## 11. 改代码时的红线

- **`list_accounts` 不能返回明文。** 给 `Account` 加了新的密文字段，要同时加进 `masked` 和 `unmask`。
- **锁库只能走 `lock_down`**，别在别处自己拼"关页面 + 放开连接"，会漏掉抹登录数据那一步。
- **导出备份只能走 `db::export_backup`**，别直接 `VACUUM INTO`，会把登录态一起带出去。
- **新增后端命令要加进 `permissions/main-window.toml`**；账号页面能调的命令只加进 `session-report.toml`，而且必须是"只往 App 里送、不读库"的。
- **别把 CSP 改回 `null`，也别加 `'unsafe-eval'`。** 改了 CSP 按 §4.4 的办法验一遍。
- **明文进剪贴板一律用 `copySecret`**（`ui.tsx`），别直接 `copyText`。
- 空密码、库不存在这两个判断在后端，别因为前端已经挡了就删掉。
