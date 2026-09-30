import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/** 主密码最短长度。跟 src-tauri/src/lib.rs 的 MIN_PASSWORD_LEN 保持一致。不做复杂度要求。 */
export const MIN_PASSWORD = 3;

/** 与 src-tauri/src/models.rs 的 Account 一一对应 */
export type ExtraField = { label: string; value: string; secret: boolean };

export type Account = {
  id: string;
  platform: string;
  owner_type: string;
  related_app: string;
  account_role: string;
  login_url: string;
  username: string;
  credential: string;
  totp_secret: string;
  backup_contact: string;
  remark: string;
  extra_fields: ExtraField[];
};

/**
 * 三个字段的内置候选。用户自己输的新值不存在这里——它跟着账号一起进库，
 * 下次由 fieldOptions() 从账号库里捞回来，所以不需要额外维护一张选项表。
 */
export const DEFAULT_PLATFORMS = ["阿里云", "腾讯云", "即构", "微信小程序", "云账户", "开发者平台", "亚马逊", "七牛云", "对象存储", "服务器", "其他"];
const GENERIC_OWNERS = ["公司账号", "个人账号"];
// "sano", "爱呗", "相亲相爱", "知伴缘",
export const DEFAULT_OWNERS = [...GENERIC_OWNERS,  "后台管理"];
export const DEFAULT_ROLES = ["子账号", "主账号", "RAM子账号", "服务号"];

const uniq = (xs: string[]) => [...new Set(xs.filter((x) => x.trim() !== ""))];

/** 候选 = 内置默认值 ∪ 账号库里已经用过的值。URL 只取同平台的历史值。 */
export const fieldOptions = (accounts: Account[], platform?: string) => ({
  platforms: uniq([...DEFAULT_PLATFORMS, ...accounts.map((a) => a.platform)]),
  // 演示时下拉框里不能冒出真实的归属名
  owners: uniq([...(demoMode ? GENERIC_OWNERS : DEFAULT_OWNERS), ...accounts.map((a) => a.owner_type)]),
  roles: uniq([...DEFAULT_ROLES, ...accounts.map((a) => a.account_role)]),
  urls: uniq(accounts.filter((a) => !platform || a.platform === platform).map((a) => a.login_url)),
});

export const emptyAccount = (): Account => ({
  id: crypto.randomUUID(),
  platform: "阿里云",
  owner_type: "公司账号",
  related_app: "",
  account_role: "子账号",
  login_url: "",
  username: "",
  credential: "",
  totp_secret: "",
  backup_contact: "",
  remark: "",
  extra_fields: [],
});

/** 与 models.rs 的 PlatformConfig 对应 */
export type PlatformConfig = {
  platform: string;
  username_selector: string;
  password_selector: string;
  trigger_event: string;
  verified: boolean;
  updated_at: string;
};

/** 与 models.rs 的 TabInfo 对应。一个账号的一个页签 */
export type TabInfo = {
  /** 页面标题，空的时候退回显示地址 */
  title: string;
  url: string;
  loaded: boolean;
  failed: boolean;
  /** 独立窗口的弹窗（window.open 带尺寸那种），点它是把那个窗口叫到前面 */
  external: boolean;
};

export type SessionInfo = {
  account_id: string;
  window_label: string;
  platform: string;
  related_app: string;
  opened_secs: number;
  /** 页面是否加载完成 */
  loaded: boolean;
  /** 打不开（限了 IP、连不上、服务器返回错误页等） */
  failed: boolean;
  /** 打不开的具体原因，直接展示给用户 */
  fail_reason: string | null;
  /** 被区域拦截切断了 */
  cut: boolean;
  /** 这个账号开着的页签；上面那几个状态说的是**当前页签** */
  tabs: TabInfo[];
  active_tab: number;
  /** 开了保活：一段时间没人动就刷新页面，免得登录过期 */
  keep_alive: boolean;
  /** 网络异常时也照样保活；没勾的话区域检测判为异常就暂停 */
  keep_alive_any_network: boolean;
};

export const isInitialized = () => invoke<boolean>("is_initialized");
export const register = (username: string, password: string) =>
  invoke<void>("register", { username, password });
/** force = 区域检测拦下之后用户明确选了"仍要解锁"（逃生口，见 lib.rs 的 unlock） */
export const unlock = (password: string, force = false) =>
  invoke<{ blocked: Verdict | null; mode: RegionMode }>("unlock", { password, force });
/** 登录页显示"当前账号是谁"。存在加密库外面，所以解锁前也读得到。 */
export const currentOwner = () => invoke<string>("current_owner");
export const lock = () => invoke<void>("lock").then(() => { demoMode = false; });

/** 演示模式：后端换到独立的假账号库，登录页全是本机模拟的。锁定即退出（见 lib.rs 的 enter_demo） */
let demoMode = false;
export const inDemo = () => demoMode;
export const enterDemo = () => invoke<void>("enter_demo").then(() => { demoMode = true; });
export const DEMO_PASSWORD = "demo";
/** 一键自动演示期间，模拟登录页填好就自己提交 */
export const setDemoTour = (on: boolean) => invoke<void>("set_demo_tour", { on });
/** 录主窗口这一块，secs 秒后自动结束（macOS 自带 screencapture） */
export const startRecording = (secs: number) => invoke<void>("start_recording", { secs });
/** keep=true 等录完、在访达里选中并返回路径；false 丢掉这段 */
export const stopRecording = (keep: boolean) => invoke<string>("stop_recording", { keep });
/** 多久无操作自动锁定，单位秒；0 = 关闭（默认） */
export const getAutoLockSecs = () => invoke<number>("get_auto_lock_secs");
export const setAutoLockSecs = (secs: number) => invoke<void>("set_auto_lock_secs", { secs });
/** 标一次"刚刚有人在动"。账号页面里的注入脚本也会调这条，所以自动锁定的空闲判断
 *  不会只看主窗口自己的操作。返回 true = 已经超时了，这次操作不续命，调用方该锁 */
export const touchActivity = () => invoke<boolean>("touch_activity");
/** 距离上一次 touchActivity 过了多少秒 */
export const idleSecs = () => invoke<number>("idle_secs");
export const listAccounts = () => invoke<Account[]>("list_accounts");
export const saveAccount = (account: Account) => invoke<void>("save_account", { account });
export const deleteAccount = (id: string) => invoke<void>("delete_account", { id });

// 网络区域检测（设计见 docs/NETWORK-GUARD.md）
export type RegionOrigin =
  | { kind: "mainland" }
  | { kind: "outside"; country: string; region: string }
  | { kind: "unknown"; reason: string };

export type Verdict = {
  origin: RegionOrigin;
  /** 外网是否直接可达 = 是否存在隧道 */
  tunnel: boolean;
  /** 系统是否配置了代理 */
  proxy: boolean;
  ip: string;
};

/** 与后端 Verdict::blocked 保持一致：分流隧道可达时，不能仅凭大陆探针放行。 */
export const regionNeedsHandling = (v: Verdict) =>
  v.origin.kind === "outside" || (v.origin.kind === "mainland" && v.tunnel);

export type RegionMode = "block" | "warn" | "off";

export type RegionEndpoint = {
  name: string;
  url: string;
  ip_path: string;
  country_path: string;
  region_path: string;
};

export type RegionState = {
  mode: RegionMode;
  verdict: Verdict;
  endpoints: RegionEndpoint[];
  /** 会话是否已被切断（页面全被导航到 about:blank，发不出任何请求） */
  cut: boolean;
  /** 后端区域哨兵的自动复查间隔，单位为秒 */
  watchSecs: number;
};

/**
 * 后端哨兵按用户设置的间隔探测网络，结果通过 `region-changed` 事件推过来。
 * `ask` 是后端算好的"要不要弹框"（见 lib.rs 的 `decide`），前端照做即可——
 * 两边各判一次迟早会不一致。异常状态持续时只会在翻转那一次为 true。
 */
export type RegionEvent = { verdict: Verdict; mode: RegionMode; ask: boolean; cut: boolean };

export const onRegionChanged = (f: (e: RegionEvent) => void) =>
  listen<RegionEvent>("region-changed", (e) => f(e.payload));

export const checkRegion = (force = false) => invoke<RegionState>("check_region", { force });
/** 主动检测并按当前模式处置运行中的会话 */
export const patrolRegion = () => invoke<RegionState>("patrol_region");
/** 立刻切断所有会话页面（warn 模式下用户选「断开」时用） */
export const cutSessions = () => invoke<number>("cut_sessions");
/** 强制重新探测；**只有确认回到大陆**才接回被切断的会话 */
export const recheckAndRestore = () => invoke<RegionState>("recheck_and_restore");
export const setRegionMode = (mode: RegionMode) => invoke<void>("set_region_mode", { mode });
export const setRegionWatchSecs = (secs: number) => invoke<void>("set_region_watch_secs", { secs });
export const setRegionEndpoints = (json: string) => invoke<number>("set_region_endpoints", { json });
export const resetRegionEndpoints = () => invoke<string>("reset_region_endpoints");

/** 出口在境外时的文字描述 */
export const regionLabel = (v: Verdict) => {
  if (v.origin.kind === "mainland") return v.tunnel ? "中国大陆 · 检测到分流代理 / VPN" : "中国大陆";
  if (v.origin.kind === "unknown") return "未知";
  const { country, region } = v.origin;
  return region && region !== country ? `${country} · ${region}` : country || "境外";
};

// 会话
export type OpenOutcome = { opened: boolean; blocked: Verdict | null; mode: RegionMode };
export const switchOrOpenAccount = (accountId: string, force = false) =>
  invoke<OpenOutcome>("switch_or_open_account", { accountId, force });
export const listActiveSessions = () => invoke<SessionInfo[]>("list_active_sessions");
/** 这个账号的保活开关，记在库里，下次打开照样生效 */
export const setKeepAlive = (accountId: string, on: boolean, anyNetwork: boolean) =>
  invoke<void>("set_keep_alive", { accountId, on, anyNetwork });
export const closeSession = (accountId: string) => invoke<void>("close_session", { accountId });
/** 重新加载页面。加载失败后的「刷新」用它 */
export const reloadSession = (accountId: string) => invoke<void>("reload_session", { accountId });
export const setPickMode = (accountId: string, on: boolean) => invoke<void>("set_pick_mode", { accountId, on });
/** 切到某个页签（target=_blank / window.open 开出来的，见 docs/BROWSER-COMPAT.md） */
export const selectSessionTab = (accountId: string, index: number) =>
  invoke<void>("select_session_tab", { accountId, index });
/** 当前页签的前进(1)/后退(-1) */
export const tabHistory = (accountId: string, delta: 1 | -1) =>
  invoke<void>("tab_history", { accountId, delta });
/** 在这个账号下开新页签打开指定地址，带着账号的登录态 */
export const openUrlInSession = (accountId: string, url: string) => invoke<void>("open_url_in_session", { accountId, url });
/** 在当前页签里打开页内查找框 */
export const findInPage = (accountId: string) => invoke<void>("find_in_page", { accountId });
/** 后端写剪贴板：前端 await 过一次之后 WKWebView 就不让写了（见 ui.tsx 的 copyText） */
export const writeClipboard = (text: string) => invoke<void>("write_clipboard", { text });
/** 30 秒后清空剪贴板，前提是那时剪贴板里还是这段（见 ui.tsx 的 copySecret） */
export const clearClipboardLater = (text: string) => invoke<void>("clear_clipboard_later", { text });
/** 在系统浏览器里打开。后端会过一遍区域检测——那边不受本 App 的拦截管 */
export const openInSystemBrowser = (url: string) => invoke<void>("open_in_system_browser", { url });
/** 新页签开不出来（数量到上限、账号页已关等）。载荷是给用户看的原因 */
export const onTabOpenFailed = (f: (reason: string) => void) =>
  listen<string>("tab-open-failed", (e) => f(e.payload));
/** 页签里的下载完成（落在设置里选的下载文件夹，重名自动加 -2）。ask = 设置里开了「每次都问」 */
export type DownloadDone = { name: string; path: string; success: boolean; ask: boolean };
export const onDownloadFinished = (f: (d: DownloadDone) => void) =>
  listen<DownloadDone>("download-finished", (e) => f(e.payload));
/** 在访达里选中下载好的文件。只认这次运行里本 App 下载 / 另存过的文件 */
export const revealDownload = (path: string) => invoke<void>("reveal_download", { path });
/** 「每次都问」：把刚下载的文件挪到保存框里选的位置，返回新路径 */
export const moveDownload = (from: string, to: string) => invoke<string>("move_download", { from, to });
/** dir = 实际存到哪；custom = 是不是自己选的；demo = 演示模式下改不了 */
export type DownloadSettings = { dir: string; custom: boolean; ask: boolean; demo: boolean };
export const getDownloadSettings = () => invoke<DownloadSettings>("get_download_settings");
/** dir 传 null = 恢复系统「下载」目录 */
export const setDownloadSettings = (dir: string | null, ask: boolean) =>
  invoke<void>("set_download_settings", { dir, ask });
/** 关掉某个页签。首个页签关不了，那是账号主页面 */
export const closeSessionTab = (accountId: string, index: number) =>
  invoke<void>("close_session_tab", { accountId, index });
export const setSessionsVisible = (visible: boolean) => invoke<void>("set_sessions_visible", { visible });
/** 指认完成，后端已把选择器写回该平台的配置。载荷是平台名 */
export const onSelectorsPicked = (f: (platform: string) => void) =>
  listen<string>("selectors-picked", (e) => f(e.payload));
export const activeSession = () => invoke<string>("active_session");
/** 把账号/密码/验证码填进"用户最后点过的那个输入框"——不依赖任何选择器。返回填的那个值，供前端放进剪贴板兜底 */
export const fillFocused = (accountId: string, field: "username" | "password" | "totp" | `extra:${number}`) =>
  invoke<string>("fill_focused", { accountId, field });
/** 把账号页面当前停的地址存成它的登录直达 URL */
export const pinCurrentUrl = (accountId: string) => invoke<string>("pin_current_url", { accountId });

// 平台适配
export const listPlatformConfigs = () => invoke<PlatformConfig[]>("list_platform_configs");
export const savePlatformConfig = (config: PlatformConfig) => invoke<void>("save_platform_config", { config });
export const exportPlatformConfigs = () => invoke<string>("export_platform_configs");
export const importPlatformConfigs = (json: string) => invoke<number>("import_platform_configs", { json });

// TOTP
/**
 * 平台配置那一行对应哪些账号。「其他」按站点存成「其他 · host」（lib.rs 的 config_key），
 * 要按登录地址的主机对上；别的平台按平台名。指认输入框和配置页判断"有没有账号可点"都用它
 */
export function accountsForConfig(accounts: Account[], key: string): Account[] {
  const [base, site] = key.split(" · ");
  const host = (url: string) => { try { return new URL(url).host; } catch { return ""; } };
  return accounts.filter((a) => a.platform === base && (!site || host(a.login_url) === site));
}
/** 清掉这个账号保存的登录状态（页面开着会被关掉），下次打开就是未登录 */
export const clearLogin = (accountId: string) => invoke<void>("clear_login", { accountId });
/** 删一条平台配置。内置平台、还有账号在用的，后端会拒 */
export const deletePlatformConfig = (platform: string) => invoke<void>("delete_platform_config", { platform });
/** 完整的一条账号（带明文）。列表里的密码、TOTP 密钥、敏感附加字段是打过码的（lib.rs 的 masked），用的时候再取 */
export const getAccount = (id: string) => invoke<Account>("get_account", { id });
/** 要复制的内容可以晚点再取：密码这类明文不常驻界面，验完主密码才去后端拿 */
export type CopySource = string | (() => Promise<string>);
export const totpCode = (accountId: string) => invoke<{ code: string; remaining: number }>("totp_code", { accountId });
/** 从 otpauth:// 链接 / 二维码图片文件取密钥；两个都不给就认剪贴板里的二维码截图 */
export const totpSecretFrom = (src: { text?: string; path?: string }) => invoke<string>("totp_secret_from", src);
/** 表单里还没保存的密钥当场出码，跟手机上的对一下 */
export const totpPreview = (secret: string) => invoke<{ code: string; remaining: number }>("totp_preview", { secret });

// 主密码 / 备份
/** 往剪贴板放明文前核一次主密码。只验钥匙，不返回任何库里的数据 */
export const verifyMasterPassword = (password: string) => invoke<void>("verify_master_password", { password });
export const changeMasterPassword = (current: string, newPassword: string) =>
  invoke<void>("change_master_password", { current, newPassword });
export const resetHint = () => invoke<{ data_dir: string; command: string }>("reset_hint");
/** 设置页的一键清空：改名归档旧库，回到"创建主账号"。跟改密码一个门槛，要当前主密码 */
export const wipeVault = (password: string) => invoke<void>("wipe_vault", { password });
export const exportBackup = () => invoke<string>("export_backup");
/** password = 这份备份当时的主密码；current = 当前账号库的主密码（整库替换，跟清空一个门槛） */
export const importBackup = (path: string, password: string, current: string) =>
  invoke<number>("import_backup", { path, password, current });
