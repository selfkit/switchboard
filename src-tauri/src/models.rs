use serde::{Deserialize, Serialize};

/// 账号之外的可填信息，如邮箱、应用 ID、主体名称。跟账号一起存在加密库中。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtraField {
    pub label: String,
    pub value: String,
    #[serde(default)]
    pub secret: bool,
}

/// 一条外部平台账号。字段对应 prototype 的 AddAccount 表单。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Account {
    pub id: String,
    pub platform: String,
    pub owner_type: String,
    pub related_app: String,
    pub account_role: String,
    pub login_url: String,
    pub username: String,
    pub credential: String,
    pub totp_secret: String,
    pub backup_contact: String,
    pub remark: String,
    #[serde(default)]
    pub extra_fields: Vec<ExtraField>,
}

/// 某个平台登录页的输入框选择器配置，对应 PlatformConfig.dc.html。
/// `trigger_event` 为 "skip" 表示这个平台走扫码之类的流程，不做自动填充。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlatformConfig {
    pub platform: String,
    pub username_selector: String,
    pub password_selector: String,
    /// 外部导入时可以省略，默认 "input+change"（最保险的一档）
    #[serde(default = "default_trigger")]
    pub trigger_event: String,
    /// 下面两个是 App 自己维护的，导入的 JSON 里没有也能解析
    #[serde(default)]
    pub verified: bool,
    #[serde(default)]
    pub updated_at: String,
}

fn default_trigger() -> String {
    "input+change".to_string()
}

/// 一个页签的状态，给前端画页签条用。
#[derive(Debug, Clone, Serialize)]
pub struct TabInfo {
    /// 页面标题，空的时候前端退回显示地址
    pub title: String,
    pub url: String,
    pub loaded: bool,
    pub failed: bool,
    /// 这是个独立窗口的弹窗（window.open 带尺寸的那种），不在主窗口那块地方
    pub external: bool,
}

/// 内存态：当前开着哪些隔离窗口。App 重启后清空。
#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub account_id: String,
    pub window_label: String,
    pub platform: String,
    pub related_app: String,
    pub opened_secs: u64,
    /// 页面有没有加载完成
    pub loaded: bool,
    /// 打不开（限了 IP、连不上、服务器返回错误页等）
    pub failed: bool,
    /// 打不开的具体原因，直接给用户看
    pub fail_reason: Option<String>,
    /// 被区域拦截切断了（已导航到 about:blank，发不出任何请求）
    pub cut: bool,
    /// 这个账号开着的页签。至少一个；loaded/failed/cut 那几项说的是**当前页签**
    pub tabs: Vec<TabInfo>,
    pub active_tab: usize,
    /// 开了保活（见 session.rs 的 keep_alive）
    pub keep_alive: bool,
    /// 网络异常时也照样保活；关着的话区域检测判为异常就暂停
    pub keep_alive_any_network: bool,
}
