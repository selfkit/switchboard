//! SessionManager：账号页面直接嵌在**主窗口**里，不再另开窗口。
//!
//! 主窗口本身就是一个 webview（跑 App 前端）。账号页面是挂在同一个窗口上的子 webview，
//! 盖在左侧导航栏右边那块区域上；App 切到「会话」页时 show，切走时 hide。
//! 所以用户看到的是：左边账号栏（前端画的），右边真实登录页（原生 webview）。
//!
//! 关键是**隔离一点没丢**：每个账号 webview 仍然各自带一份 cookie 存储。
//! 两套机制，因为两个平台的 WebView 根本不是一个东西：
//! - Windows (WebView2) / Linux (WebKitGTK)：`data_directory`，每个账号一个目录
//! - macOS (WKWebView)：**完全无视 data_directory**，只认 `WKWebsiteDataStore`
//!   的 identifier，所以走 `data_store_identifier`（需要 macOS 14+）

use crate::adapter;
use crate::models::{Account, SessionInfo, TabInfo};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::webview::cookie::Cookie;
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Webview, WebviewBuilder, WebviewUrl,
    Window, WindowEvent,
};

/// 主窗口左侧导航栏宽度，账号页面从这里开始铺
const SIDEBAR_W: f64 = 224.0;
/// 顶部留给 App 自己画的一条（右上角的品牌标就在这儿）。
/// 账号页面是原生 webview，会盖住网页画的一切，所以只能给它让出这块地方。
const TOPBAR_H: f64 = 40.0;
/// 顶栏下面那条页签条的高度。**必须和前端 Layout.tsx 的 TABS_H 一致**，
/// 否则页签条和页面之间会露一条缝或者互相盖住（有 `frontend_layout_constants_match` 钉着）。
/// 固定占位、不按"有没有第二个页签"动态收放：高度一变 webview 整块跳，比省掉这 34px 难受。
const TABS_H: f64 = 34.0;
/// 多久没等到"页面加载完成"就算加载失败。
/// wry 没有导航失败的回调（没有 didFailProvisionalNavigation 这类东西），
/// 只能用超时判定。宝塔那种限了 IP 的站会一直连不上，不设这个就永远白板。
const LOAD_TIMEOUT_SECS: u64 = 12;
const MAIN_LABEL: &str = "main";
/// 一个账号最多开多少个页签。页签是页面自己开的，得有个上限
const MAX_TABS: usize = 12;
/// 打开或刷新之后多久内的页面才自动填充。登录就发生在这段时间里；
/// 登录后在控制台里到处点，页面上的"修改密码"之类的框不该再被填上账号密码
const FILL_WINDOW_SECS: u64 = 120;

/// 一个账号里的一个页签 = 一个子 webview。
///
/// 页面自己那些状态（加载没加载、打不开、停在哪、有没有被切断）全是**按页签**的：
/// 同一个账号的两个页签可能一个好着一个 404。账号级的东西（平台、归属、cookie 仓）
/// 留在 [`SessionState`] 上。
pub struct Tab {
    /// 这个页签对应的子 webview 的 label
    pub label: String,
    /// 本次导航开始的时间。刷新会重置它，用来算加载超时（跟 opened_at 分开，
    /// 后者要给"已打开 N 分钟"用）
    pub nav_at: Instant,
    /// 页面有没有加载完成。由 on_page_load 的 Finished 置位
    pub loaded: Arc<AtomicBool>,
    /// 服务器回过话了（导航已提交、页面开始渲染）。由 on_page_load 的 Started 置位。
    /// 加载超时只管"压根连不上"：带宽小的服务器上一个 1MB 的 CSS 要下十几秒，
    /// 这时 Finished 迟迟不来，但页面是好的——浏览器也只是转圈，不该判成打不开
    pub committed: Arc<AtomicBool>,
    /// 页面自己报回来的失败原因（HTTP 4xx/5xx、纯白板）。见 `adapter::page_probe_js`
    pub fail: Option<String>,
    /// 页面最后停在哪。由 on_page_load 从 payload 里抄下来——
    /// **不能**改用 `webview.url()` 去现问，见 `url_is_safe` 那段。
    /// 区域拦截把页面切到 about:blank 之后，靠它导航回来。
    pub last_url: Arc<Mutex<String>>,
    /// 页面标题，页签上显示。由 on_document_title_changed 写进来
    pub title: Arc<Mutex<String>>,
    /// 自动填充从什么时候起算（打开 / 刷新时重置），见 `FILL_WINDOW_SECS`
    pub fill_armed: Arc<Mutex<Instant>>,
    /// 被区域拦截切断了。切断 = 已经导航到 about:blank，页面一个请求都发不出去
    pub cut: bool,
    /// 当前是不是显示着。只在状态变化时才调 show/hide，避免每次轮询都闪
    pub shown: bool,
    /// 是否正处在"手动指认输入框"模式，只有这时才接受页面回传的选择器
    pub picking: bool,
    /// 这个页签是不是一个**独立窗口**（`window.open` 带尺寸的真弹窗，见 docs/BROWSER-COMPAT.md）。
    /// 独立窗口不参与主窗口那块区域的摆放和显示隐藏，但**照样登记在这里**——
    /// 区域拦截靠这张表逐个切断，漏登记就等于开了个切不断的窗口。
    pub external: bool,
    /// 真正显示过一个页面（about:blank 不算）。页签开出来的第一次导航就变成了下载的话，它一直是 false
    pub seen_page: Arc<AtomicBool>,
    /// 专门为一次下载开出来的空页签 / 空弹窗，下载结束就关掉，见 `SessionState::mark_download_only`
    pub download_only: bool,
    /// 开出这个页签时用户停在哪个页签（label）。下载专用的空页签要把用户送回那里
    pub return_to: Option<String>,
}

impl Tab {
    /// 打不开的原因，能用就是 `None`。失败时把 webview 藏起来，
    /// 好让前端能在那块区域画错误提示——原生 webview 盖在网页之上，不藏就只能看白板。
    ///
    /// 两条判据缺一不可：
    /// - **超时**：导航压根没完成（DNS 挂了、连不上、被防火墙丢包）
    /// - **页面自报**：导航成功了但内容是错误页。服务器回 4xx 只要 69ms，
    ///   超时判定永远轮不到它——这正是"限了 IP 的宝塔面板"那个场景
    pub fn failure(&self) -> Option<String> {
        // 下载专用的空页签本来就不会有页面，不算打不开（Windows 上它还会报一次"加载完成"、让探针判成白板）
        if self.download_only {
            return None;
        }
        if let Some(r) = &self.fail {
            return Some(r.clone());
        }
        if !self.loaded.load(Ordering::Relaxed)
            && !self.committed.load(Ordering::Relaxed)
            && self.nav_at.elapsed().as_secs() >= LOAD_TIMEOUT_SECS
        {
            return Some(format!("等了 {LOAD_TIMEOUT_SECS} 秒没有响应"));
        }
        None
    }

    pub fn failed(&self) -> bool {
        self.failure().is_some()
    }

    /// 能不能安全地问 WKWebView 要当前地址。
    ///
    /// ⚠️ 导航没提交时 `WKWebView.URL` 是 **nil**，而 wry 是直接
    /// `webview.URL().unwrap()`（wry-0.55.1 `wkwebview/mod.rs:1349`）。
    /// 这个 unwrap 跑在**事件循环线程**上（tauri 的 `url()` 走 `webview_getter!` 派发），
    /// 调用方 `catch_unwind` 接不住，而且 release 是 `panic = "abort"` ——
    /// 结果就是整个 App 当场 SIGABRT，连 cookie 快照都来不及存。所以只能提前挡。
    pub fn url_is_safe(&self) -> bool {
        self.loaded.load(Ordering::Relaxed)
    }
}

/// 一个账号的会话：一组页签 + 账号级信息。
pub struct SessionState {
    /// 至少一个。`tabs[0]` 的 label 永远是 `label_for(account_id)`，
    /// 不给关（关掉等于关账号，那件事走侧栏的 ×），这样 `label_for` 的调用方都还成立。
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
    /// 下一个页签的序号。**只增不复用**——复用会撞上刚关掉那个 webview 的残留 label
    pub next_seq: u32,
    pub platform: String,
    pub related_app: String,
    pub opened_at: Instant,
}

impl SessionState {
    pub fn active(&self) -> &Tab {
        // active_tab 由 select_tab / close_tab 维护，越界只可能是自己写错了，
        // 但这条路上崩一次等于整个 App 没了（release 是 panic = abort），所以退回第一个
        self.tabs.get(self.active_tab).unwrap_or(&self.tabs[0])
    }

    fn active_mut(&mut self) -> &mut Tab {
        let i = if self.active_tab < self.tabs.len() { self.active_tab } else { 0 };
        &mut self.tabs[i]
    }

    fn tab_of(&mut self, label: &str) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.label == label)
    }

    /// 页签里的下载开始了。这个页签要是页面刚开出来、第一次导航就变成了下载（`window.open` /
    /// `target=_blank` 指向一个下载地址），它里面什么都不会有：WebKit 把导航转成下载后既不报"提交"也不报"加载完成"，
    /// 不管的话 12 秒后被加载超时判成"打不开"，整块区域变成错误提示——看上去就是"一下载页面就崩了"。
    /// 跟浏览器一样处理：用户送回原来那个页签，这个空页签等下载结束再关（见 `download_finished`）。
    ///
    /// 返回 true = 这是个下载专用页签。首个页签是账号主页面，永远不算；显示过页面的页签也不算
    fn mark_download_only(&mut self, label: &str) -> bool {
        let Some(i) = self.tabs.iter().position(|t| t.label == label) else { return false };
        let t = &mut self.tabs[i];
        if t.download_only {
            return true;
        }
        if i == 0 || t.seen_page.load(Ordering::Relaxed) {
            return false;
        }
        t.download_only = true;
        t.fail = None;
        t.loaded.store(true, Ordering::Relaxed); // 页签条上别一直挂着"加载中"
        if let Ok(mut title) = t.title.lock() {
            *title = "下载中…".into();
        }
        let back = t.return_to.clone();
        if self.active_tab == i {
            self.active_tab = back
                .and_then(|l| self.tabs.iter().position(|x| x.label == l && !x.external))
                .unwrap_or(0);
        }
        true
    }
}

#[derive(Default)]
pub struct Sessions {
    pub map: Mutex<HashMap<String, SessionState>>,
    /// App 当前是不是停在「会话」页；不是的话所有账号页面都藏起来
    pub visible: Mutex<bool>,
    /// 当前显示的是哪个账号（存 account_id）。
    /// 以前存的是 webview 的 label，一个账号多页签之后 label 不再唯一标识账号了。
    pub active: Mutex<String>,
}

pub fn label_for(account_id: &str) -> String {
    let safe: String = account_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
        .collect();
    format!("acct-{safe}")
}

/// 第 n 个页签的 label。`seq = 0` 就是首个页签，跟 `label_for` 完全相同 ——
/// 首个页签的 label 不能变，`report_picked_selectors` 和 capability 的 `acct-*` 都指着它。
fn tab_label(account_id: &str, seq: u32) -> String {
    let base = label_for(account_id);
    if seq == 0 { base } else { format!("{base}-t{seq}") }
}

/// 弹窗窗口的 label。仍然以 `acct-` 开头 —— capability 的 `webviews: ["acct-*"]`
/// 是按 webview label 匹配的（跟窗口 label 是**或**关系），所以弹窗不用改权限配置。
fn popup_label(account_id: &str, seq: u32) -> String {
    format!("{}-p{seq}", label_for(account_id))
}

fn profile_dir(app: &AppHandle, account_id: &str) -> Result<PathBuf, String> {
    // 跟账号库同一个根目录，所以开发版的隔离数据也自动分开
    let dir = crate::data_dir(app)?.join("profiles").join(account_id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// 从账号 id 派生一个稳定的 16 字节标识（账号 id 本身就是 uuid，取它的 hex 即可）。
/// 同一个账号每次都得到同一份 data store，cookie 才留得住。
fn store_identifier(account_id: &str) -> [u8; 16] {
    let hex: Vec<u8> = account_id
        .bytes()
        .filter(|b| b.is_ascii_hexdigit())
        .take(32)
        .collect();
    let mut out = [0u8; 16];
    for (i, pair) in hex.chunks(2).enumerate() {
        let hi = (pair[0] as char).to_digit(16).unwrap_or(0) as u8;
        let lo = pair.get(1).and_then(|c| (*c as char).to_digit(16)).unwrap_or(0) as u8;
        out[i] = hi << 4 | lo;
    }
    out
}

fn main_window(app: &AppHandle) -> Result<Window, String> {
    app.get_window(MAIN_LABEL).ok_or_else(|| "主窗口不见了".to_string())
}

/// 窗口当前的逻辑尺寸（webview 的摆放用逻辑像素，窗口给的是物理像素）
fn logical_size(win: &Window) -> Result<(f64, f64), String> {
    let phys = win.inner_size().map_err(|e| e.to_string())?;
    let scale = win.scale_factor().map_err(|e| e.to_string())?;
    Ok((phys.width as f64 / scale, phys.height as f64 / scale))
}

fn body_rect(win: &Window) -> Result<(LogicalPosition<f64>, LogicalSize<f64>), String> {
    let (w, h) = logical_size(win)?;
    let top = TOPBAR_H + TABS_H;
    Ok((
        LogicalPosition::new(SIDEBAR_W, top),
        LogicalSize::new((w - SIDEBAR_W).max(0.0), (h - top).max(0.0)),
    ))
}

/// 摆正所有账号页面，并按「当前是否在会话页 + 谁是 active」决定显示哪个。
pub fn refresh_layout(app: &AppHandle) {
    apply_layout(app, true);
}

/// `reposition=false` 只把显示/隐藏对齐到当前状态，不动位置。
/// 加载超时是没有任何事件的（wry 没有导航失败回调），靠前端轮询 `list` 时顺手对一次，
/// 否则超时的白板一直盖着前端画的"打不开 / 刷新重试"。状态没变时是空操作。
fn apply_layout(app: &AppHandle, reposition: bool) {
    let rect = if reposition {
        let Ok(win) = main_window(app) else { return };
        let Ok(r) = body_rect(&win) else { return };
        Some(r)
    } else {
        None
    };

    let sessions = app.state::<Sessions>();
    let visible = sessions.visible.lock().map(|v| *v).unwrap_or(false);
    let active = sessions.active.lock().map(|v| v.clone()).unwrap_or_default();
    let Ok(mut map) = sessions.map.lock() else { return };
    for (id, s) in map.iter_mut() {
        let is_active_account = *id == active;
        let active_tab = s.active_tab;
        for (i, t) in s.tabs.iter_mut().enumerate() {
            // 独立窗口自己有窗框，别去摆它的位置，也别 show/hide 它
            if t.external {
                continue;
            }
            let Some(v) = app.get_webview(&t.label) else { continue };
            if let Some((pos, size)) = rect {
                let _ = v.set_position(pos);
                let _ = v.set_size(size);
            }
            // 加载失败/被切断的页面要藏起来，否则它那块白板盖住前端画的提示。
            // 原生 webview 永远盖在主窗口网页之上，HTML 弹层压不住它。
            // 非当前页签的也一律藏起来——它们是同一块地方上的好几层。
            let want = visible && is_active_account && i == active_tab && !t.failed() && !t.cut;
            if want != t.shown {
                let _ = if want { v.show() } else { v.hide() };
                t.shown = want;
            }
        }
    }
}

/// App 切到 / 切离「会话」页时调用。
pub fn set_visible(app: &AppHandle, visible: bool) -> Result<(), String> {
    *app.state::<Sessions>().visible.lock().map_err(|e| e.to_string())? = visible;
    refresh_layout(app);
    Ok(())
}

/// 主窗口拖大拖小时要重排。只挂一次。
pub fn watch_main_window(app: &AppHandle) {
    let Ok(win) = main_window(app) else { return };
    let app2 = app.clone();
    win.on_window_event(move |e| {
        if matches!(e, WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }) {
            refresh_layout(&app2);
        }
    });
}

/// 读出这个账号当前的全部 cookie，序列化成 Set-Cookie 字符串数组。
/// 登录态 cookie 大多是**会话 cookie**（不带过期时间），WebView 一关就没了——
/// 这是浏览器的规范行为，不是 bug。所以退出前自己抄一份，下次开回去。
pub fn read_cookies(app: &AppHandle, account_id: &str) -> Vec<String> {
    // cookie 是 data store 级的，同一账号所有页签共享一份，取首个页签即可。
    // 必须在关掉最后一个页签**之前**抄。
    let Ok(v) = first_webview(app, account_id) else { return Vec::new() };
    v.cookies()
        .map(|cs| cs.iter().map(|c| c.to_string()).collect())
        .unwrap_or_default()
}

/// 把上次抄下来的 cookie 塞回去。必须在页面开始加载**之前**做，
/// 否则第一个请求不带 cookie，照样是未登录。
fn restore_cookies(v: &Webview, cookies: &[String]) {
    for raw in cookies {
        if let Ok(c) = Cookie::parse(raw.clone()) {
            let _ = v.set_cookie(c);
        }
    }
}

/// 下载落哪儿：设置里选的文件夹，没选就是系统的「下载」目录（演示模式另放，见 `downloads_dir`），重名就加 `-2`、`-3`。
///
/// 不弹保存对话框——WKWebView 那套要自己实现 NSSavePanel，而备案/证书这类文件
/// 用户要的只是"存下来能找到"。目录固定 + 重名不覆盖，够用且不会悄悄吃掉旧文件。
fn download_path(app: &AppHandle, suggested: &str, url: &tauri::Url) -> Option<PathBuf> {
    let dir = crate::downloads_dir(app).ok()?;
    let name = download_name(suggested, url);
    let mut path = dir.join(&name);
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    let mut n = 2;
    while path.exists() && n < 100 {
        path = dir.join(format!("{stem}-{n}{ext}"));
        n += 1;
    }
    // 编号用完了还重名就不指定落点，交给 WebKit 自己起名——总比悄悄盖掉 -99 那个强
    if path.exists() {
        return None;
    }
    Some(path)
}

/// 挑一个能直接拼进目录的文件名：先用 WebKit 给的建议名（Content-Disposition / `a[download]`），
/// 没有再退回下载地址的最后一段——`blob:`、`/api/export?id=1` 这类地址里没有像样的名字，
/// 只看地址就会存成一个没扩展名的 `download`，用户根本认不出是哪个文件。
///
/// 这两个名字都**来自远端**，不能原样拿去 join：
/// - `..` / `.` 会把落点挪出「下载」目录（`dir.join("..")` 就是上一级）
/// - 路径分隔符和控制字符同理，一律换成 `-`
/// 都拿不到像样的名字就叫 `download`。
fn download_name(suggested: &str, url: &tauri::Url) -> String {
    let from_url = url.path_segments().and_then(|mut s| s.next_back()).unwrap_or("");
    [suggested, from_url]
        .into_iter()
        .map(|raw| clean_name(raw, cfg!(windows)))
        .find(|n| !n.is_empty())
        .unwrap_or_else(|| "download".to_string())
}

/// Windows 设备名：`NUL.txt` 之类的文件名会被当成设备，写进去的内容直接消失
const WIN_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
    "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// 清洗一个远端给的文件名。`windows` 为真时再按 Windows 的规矩多清几样：
/// `< > " | ? *` 建不了文件，末尾的空格和点会被系统悄悄去掉，设备名要改掉
fn clean_name(raw: &str, windows: bool) -> String {
    // 先解码再清洗，顺序反了等于没清：%2F 解出来就是真斜杠
    let decoded = percent_encoding::percent_decode_str(raw.trim()).decode_utf8_lossy();
    let bad = if windows { "/\\:<>\"|?*" } else { "/\\:" };
    let cleaned: String = decoded.chars().map(|c| if c.is_control() || bad.contains(c) { '-' } else { c }).collect();
    let mut name = cleaned.trim_matches('.').trim().to_string();
    if windows {
        name = name.trim_end_matches(['.', ' ']).to_string();
        let stem = name.split('.').next().unwrap_or("").trim().to_ascii_uppercase();
        if WIN_RESERVED.contains(&stem.as_str()) {
            name.insert(0, '_');
        }
    }
    name
}

/// 页签和弹窗共用的下载处理。
///
/// macOS 的 `Finished` 事件**不带路径**（API 限制），所以在 `Requested` 时把我们选的
/// 路径记在 `last` 里，完成时报给前端——否则用户只知道"下完了"，不知道文件去哪了。
fn handle_download(
    app: &AppHandle,
    label: &str,
    last: &Arc<Mutex<PathBuf>>,
    event: tauri::webview::DownloadEvent<'_>,
) -> bool {
    match event {
        tauri::webview::DownloadEvent::Requested { url, destination } => {
            // wry 递进来的 destination 已经是「下载目录 + WebKit 建议名」，只取它的文件名
            let suggested = destination
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if let Some(p) = download_path(app, &suggested, &url) {
                if let Ok(mut l) = last.lock() {
                    *l = p.clone();
                }
                crate::remember_download(&p);
                *destination = p;
            }
            download_started(app, label);
        }
        tauri::webview::DownloadEvent::Finished { success, .. } => {
            let path = last.lock().map(|p| p.clone()).unwrap_or_default();
            let _ = app.emit_to(
                tauri::EventTarget::labeled(MAIN_LABEL),
                "download-finished",
                DownloadDone {
                    name: path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    path: path.display().to_string(),
                    success,
                    ask: crate::download_ask(app),
                },
            );
            download_finished(app, label);
        }
        _ => (),
    }
    true // 让下载继续
}

fn download_started(app: &AppHandle, label: &str) {
    let changed = {
        let sessions = app.state::<Sessions>();
        let Ok(mut map) = sessions.map.lock() else { return };
        map.values_mut().any(|s| s.mark_download_only(label))
    };
    if changed {
        refresh_layout(app);
    }
}

/// 下载结束（成功失败都算）：下载专用的空页签 / 空弹窗关掉。
///
/// 不能在下载开始时就关：WKDownload 的 delegate 是弱引用、挂在发起它的 webview 上，webview 一关，
/// 「下载完成」就再也收不到，用户看不到文件存哪了。也不在这个回调里当场关——还在 WebKit 的回调里
/// 就销毁发起它的 webview 不安全，挪到别的线程稍等一下再关
fn download_finished(app: &AppHandle, label: &str) {
    let (app, label) = (app.clone(), label.to_string());
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        let only = {
            let sessions = app.state::<Sessions>();
            let Ok(map) = sessions.map.lock() else { return };
            map.values().flat_map(|s| s.tabs.iter()).any(|t| t.label == label && t.download_only)
        };
        if only {
            if let Some(external) = remove_tab_by_label(&app, &label) {
                let _ = close_view(&app, &label, external);
            }
        }
    });
}

/// 下载完成的通知，发给主窗口前端。
#[derive(Clone, serde::Serialize)]
struct DownloadDone {
    name: String,
    path: String,
    success: bool,
    /// 设置里开了「每次都问」：前端弹保存框，选好后调 move_download 挪过去
    ask: bool,
}

/// 近似"同一个站"（可注册域名）：取主机名最后两段；倒数第二段是 com/net/org/gov/edu/co/ac
/// 且顶级域是两个字母的国家码时（com.cn、co.uk）取最后三段。跟 adapter.rs 里 JS 的 `site()` 同一套规则。
/// ponytail: 没带公共后缀表，github.io 这类多租户后缀会被当成同一个站；真有登录页落在那种域名上再换 publicsuffix
fn site_of(host: &str) -> String {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let parts: Vec<&str> = host.split('.').collect();
    let n = parts.len();
    let take = if n > 2
        && parts[n - 1].len() == 2
        && matches!(parts[n - 2], "com" | "net" | "org" | "gov" | "edu" | "co" | "ac")
    {
        3
    } else {
        2
    };
    parts[n.saturating_sub(take)..].join(".")
}

/// 手动填密码的站点校验：同一个域名（按 `site_of` 算，子域名算一个）或同一个 IP 就放行，协议和端口不管。
/// 比 `fill_allowed` 松：这是用户自己点的，同一台机器上换了个端口的服务（宝塔面板和它上面的站）也该能填
fn same_host(login: &tauri::Url, page: &tauri::Url) -> bool {
    match (login.domain(), page.domain()) {
        (Some(a), Some(b)) if a.contains('.') => site_of(a) == site_of(b),
        _ => login.host_str().is_some() && login.host_str() == page.host_str(),
    }
}

/// 自动填充只在跟登录直达 URL **同一个站**的页面上跑。
///
/// 填充脚本里是明文密码，而同一个页签是会跳走的（控制台里点了个站外链接、SSO 跳转）。
/// 不挡的话每换一页都 eval 一遍：别的站只要有个 `input[type=password]` 就会被填上，
/// 页面自己把 `window.__sb` 换成自己的函数更是直接拿到密码。
/// 同站而不是同源：`account.aliyun.com` 跳 `signin.aliyun.com` 这种正常流程得留着；
/// 协议也要一致，https 登录页的密码不往 http 页里填。
/// IP 和单段主机名（自建的宝塔/Graylog）没有"站"的概念，要求协议+主机+端口完全一致。
fn fill_allowed(login: &tauri::Url, page: &tauri::Url) -> bool {
    if login.scheme() != page.scheme() {
        return false;
    }
    match (login.domain(), page.domain()) {
        (Some(a), Some(b)) if a.contains('.') => site_of(a) == site_of(b),
        _ => login.origin() == page.origin(),
    }
}

/// 建一个页签的 webview 并挂到主窗口上。
///
/// 首个页签和 `window.open` / `target=_blank` 开出来的新页签**走同一条路**：
/// 同一份 profile + 同一个 data store（登录态天然共享，新页签就是"同一个浏览器的新标签"）、
/// 同一套注入脚本、同一个 on_new_window（新页签里还能再开新页签）。
///
/// 先开在 about:blank 再 navigate：cookie 必须赶在真正的请求发出去之前塞进 store，
/// 直接用目标 URL 建 webview 的话导航已经启动了，来不及。
fn build_tab(
    app: &AppHandle,
    account_id: &str,
    seq: u32,
    target: tauri::Url,
    autofill: bool,
    cookies: &[String],
) -> Result<Tab, String> {
    let label = tab_label(account_id, seq);
    let win = main_window(app)?;
    let (pos, size) = body_rect(&win)?;
    let blank = "about:blank".parse().map_err(|_| "about:blank 解析失败".to_string())?;

    let loaded = Arc::new(AtomicBool::new(false));
    let loaded_cb = loaded.clone();
    let committed = Arc::new(AtomicBool::new(false));
    let committed_cb = committed.clone();
    let last_url = Arc::new(Mutex::new(target.to_string()));
    let last_url_cb = last_url.clone();
    let title = Arc::new(Mutex::new(String::new()));
    let title_cb = title.clone();
    let fill_id = account_id.to_string();
    let fill_armed = Arc::new(Mutex::new(Instant::now()));
    let fill_armed_cb = fill_armed.clone();
    let seen_page = Arc::new(AtomicBool::new(false));
    let seen_cb = seen_page.clone();
    let dl_label = label.clone();

    let popup_app = app.clone();
    let popup_id = account_id.to_string();
    let dl_app = app.clone();
    let dl_last = Arc::new(Mutex::new(PathBuf::new()));

    let builder = WebviewBuilder::new(&label, WebviewUrl::External(blank))
        .data_directory(profile_dir(app, account_id)?)
        .data_store_identifier(store_identifier(account_id))
        // 常驻：填充 / 指认 / 记住最后点过的输入框，全靠它。
        // 必须**每个 frame 都注入**——阿里云腾讯云这类站的登录框在跨域 iframe 里，
        // 只注主文档的话点击记不到、指认点不中、自动填充也找不到框
        .initialization_script_for_all_frames(adapter::agent_js())
        // Tauri 默认给每个 webview 装自己的拖放接收器，OS 的拖放事件被它接走，
        // 页面里的 dragenter/dragover/drop 根本不触发——备案页那种"拖拽到此区域"
        // 于是永远没反应。我们不用 Tauri 的 DragDrop 事件，关掉，把拖放还给页面。
        .disable_drag_drop_handler()
        // 页签标题
        .on_document_title_changed(move |_, t| {
            if let Ok(mut cur) = title_cb.lock() {
                *cur = t;
            }
        })
        // target=_blank / window.open：wry 不设这个钩子的话 WKWebView 直接把请求丢掉，
        // 表现就是"点了毫无反应"。我们拒掉原生那条路（它会开一个逃出 Sessions 管理、
        // 区域拦截切不断的独立窗口），改成在同一账号下自己开一个页签。
        // 详见 docs/BROWSER-COMPAT.md 的选型表。
        .on_new_window(move |url, features| {
            // 带尺寸的才是真弹窗（OAuth 那种 window.open(w,h)，页面还攥着返回的句柄
            // 等 postMessage 回来）。这种必须让 WKWebView 自己接管新 webview，
            // 否则 window.opener 那条链就断了。不带尺寸的（target=_blank 居多）
            // 走 App 内页签，留在我们自己的壳里。
            if features.size().is_some() {
                match open_popup_window(&popup_app, &popup_id, &url, features) {
                    Ok(window) => return tauri::webview::NewWindowResponse::Create { window },
                    // 开不出来就退回页签：总比"点了没反应"好
                    Err(_) => {}
                }
            }
            spawn_tab(&popup_app, &popup_id, url);
            tauri::webview::NewWindowResponse::Deny
        })
        // 下载：macOS 的 Finished 事件**不带路径**（API 限制），所以自己记一份，
        // 完成时把我们选的那个路径报给前端，否则用户只知道"下完了"但不知道在哪
        .on_download(move |_, event| handle_download(&dl_app, &dl_label, &dl_last, event))
        // 这个回调**永远要挂**（不只在有填充脚本时）：它是判断
        // "页面到底加载出来没有"的唯一信号，没有它就分不清白板和正常空页
        .on_page_load(move |webview, payload| {
            // about:blank 是我们自己的中转页，不算加载成功。
            // 用 payload 给的 URL，**不要**调 webview.url()：那个要派发回事件循环线程，
            // 而这里就跑在事件循环线程上，等于自己等自己。
            if payload.url().as_str() == "about:blank" {
                return;
            }
            if payload.event() != tauri::webview::PageLoadEvent::Finished {
                committed_cb.store(true, Ordering::Relaxed);
                return;
            }
            if let Ok(mut u) = last_url_cb.lock() {
                *u = payload.url().to_string();
            }
            loaded_cb.store(true, Ordering::Relaxed);
            seen_cb.store(true, Ordering::Relaxed);
            // 导航"完成"不等于页面能用：4xx/5xx 也是完成。让页面自己回报一次
            let _ = webview.eval(adapter::page_probe_js());
            let armed = fill_armed_cb.lock().map(|t| t.elapsed().as_secs() < FILL_WINDOW_SECS).unwrap_or(false);
            if autofill && armed {
                // 每次现取：页面开着的时候账号密码、平台选择器都可能被改过（编辑账号、指认输入框）
                if let Some((login, js)) = crate::autofill_for(webview.app_handle(), &fill_id) {
                    if fill_allowed(&login, payload.url()) {
                        let _ = webview.eval(&js);
                    }
                }
            }
        });

    let view = win.add_child(builder, pos, size).map_err(|e| e.to_string())?;
    restore_cookies(&view, cookies);
    view.navigate(target).map_err(|e| e.to_string())?;

    Ok(Tab {
        label,
        nav_at: Instant::now(),
        loaded,
        committed,
        fail: None,
        last_url,
        title,
        fill_armed,
        cut: false,
        shown: false,
        picking: false,
        external: false,
        seen_page,
        download_only: false,
        return_to: None,
    })
}

/// 在账号下开一个新页签，给 `on_new_window` 回调用。
///
/// ⚠️ 那个回调跑在**事件循环线程**上，而 add_child 是往事件循环派发消息再等回执——
/// 在那儿同步建**子 webview** 等于自己等自己，必须甩到别的线程去建。
/// （开**窗口**不受这条限制：create_window 在主线程上是内联执行的，
///   所以 open_popup_window 可以同步返回。）
/// 开不出来必须说话，静默失败的表现就是"点了链接毫无反应"。
fn spawn_tab(app: &AppHandle, account_id: &str, url: tauri::Url) {
    let (app, id) = (app.clone(), account_id.to_string());
    std::thread::spawn(move || {
        if let Err(e) = open_tab(&app, &id, url) {
            let _ = app.emit_to(tauri::EventTarget::labeled(MAIN_LABEL), "tab-open-failed", e);
        }
    });
}

/// 真弹窗：开一个**独立窗口**承接 `window.open`，并把它登记成这个账号的一个页签。
///
/// 为什么能在 `on_new_window` 回调里同步建窗口：`create_window` 走
/// `send_user_message`，它发现自己已经在主线程就**内联执行**
/// （tauri-runtime-wry:239）；只有 `create_webview` 明确要求"必须从别的线程调"。
/// 而 `NewWindowResponse::Create` 要的正好是窗口。
///
/// `window_features()` 一行把平台差异都接上了：macOS 共享 `WKWebViewConfiguration`、
/// Windows 共享 environment、Linux 设 related view，并套用弹窗要的尺寸和位置。
/// 共享 configuration 意味着 **data store 和注入脚本都跟着过去**，登录态不用另外搬。
fn open_popup_window(
    app: &AppHandle,
    account_id: &str,
    url: &tauri::Url,
    features: tauri::webview::NewWindowFeatures,
) -> Result<tauri::WebviewWindow, String> {
    let seq = {
        let sessions = app.state::<Sessions>();
        let mut map = sessions.map.lock().map_err(|e| e.to_string())?;
        let s = map.get_mut(account_id).ok_or("这个账号的页面没开着")?;
        if s.tabs.len() >= MAX_TABS {
            return Err(format!("这个账号已经开了 {MAX_TABS} 个页签/弹窗"));
        }
        let seq = s.next_seq;
        s.next_seq += 1;
        seq
    };
    let label = popup_label(account_id, seq);
    let title = Arc::new(Mutex::new(String::new()));
    let title_cb = title.clone();
    let seen_page = Arc::new(AtomicBool::new(false));

    // 起在 about:blank，真正的地址由 WKWebView 自己导航过去（它接管了这次 window.open）
    let window = tauri::WebviewWindowBuilder::new(
        app,
        &label,
        WebviewUrl::External("about:blank".parse().map_err(|_| "about:blank 解析失败")?),
    )
    .window_features(features)
    // 跟账号页面同一个道理：Tauri 自己的拖放接收器会把 OS 拖放接走，
    // 弹窗里要是有上传区就又拖不进去了
    .disable_drag_drop_handler()
    // 弹窗里也可能点下载（有些平台把证书/模板放在弹出的说明页里）
    .on_download({
        let (dl_app, dl_label) = (app.clone(), label.clone());
        let dl_last = Arc::new(Mutex::new(PathBuf::new()));
        move |_, event| handle_download(&dl_app, &dl_label, &dl_last, event)
    })
    // 只用来认"这个弹窗有没有显示过页面"：window.open 直接开一个下载地址的弹窗，下载完要关掉
    .on_page_load({
        let seen = seen_page.clone();
        move |_, p| {
            if p.event() == tauri::webview::PageLoadEvent::Finished && p.url().as_str() != "about:blank" {
                seen.store(true, Ordering::Relaxed);
            }
        }
    })
    // 弹窗里的 target=_blank 也得有人接，不然又是点了没反应；开成账号下的页签
    .on_new_window({
        let (app, id) = (app.clone(), account_id.to_string());
        move |url, _| {
            spawn_tab(&app, &id, url);
            tauri::webview::NewWindowResponse::Deny
        }
    })
    .title(url.as_str())
    .on_document_title_changed(move |w, t| {
        let _ = w.set_title(&t);
        if let Ok(mut cur) = title_cb.lock() {
            *cur = t;
        }
    })
    .build()
    .map_err(|e| e.to_string())?;

    // 用户关掉弹窗窗口时把这条页签也摘掉，否则区域拦截以后会去操作一个已经没了的 webview
    let gone_app = app.clone();
    let gone_label = label.clone();
    window.on_window_event(move |e| {
        if matches!(e, WindowEvent::Destroyed) {
            remove_tab_by_label(&gone_app, &gone_label);
        }
    });

    {
        let sessions = app.state::<Sessions>();
        let mut map = sessions.map.lock().map_err(|e| e.to_string())?;
        let s = map.get_mut(account_id).ok_or("这个账号的页面没开着")?;
        s.tabs.push(Tab {
            label,
            nav_at: Instant::now(),
            // 弹窗里没有我们的探针回传（页面自检脚本不一定跟过去），
            // 所以直接按"已加载"记账，免得被加载超时判成打不开
            loaded: Arc::new(AtomicBool::new(true)),
            committed: Arc::new(AtomicBool::new(true)),
            fail: None,
            last_url: Arc::new(Mutex::new(url.to_string())),
            title,
            fill_armed: Arc::new(Mutex::new(Instant::now())),
            cut: false,
            shown: true,
            picking: false,
            external: true,
            seen_page,
            download_only: false,
            return_to: None,
        });
    }
    Ok(window)
}

/// 按 label 从表里摘掉一条页签（弹窗窗口被用户关掉、下载专用页签下完时用），不动它背后的 webview。
/// 摘掉了返回它是不是弹窗，调用方据此决定要不要再关 webview / 窗口。首个页签不摘
fn remove_tab_by_label(app: &AppHandle, label: &str) -> Option<bool> {
    let mut removed = None;
    {
        let sessions = app.state::<Sessions>();
        let Ok(mut map) = sessions.map.lock() else { return None };
        for s in map.values_mut() {
            let Some(i) = s.tabs.iter().position(|t| t.label == label) else { continue };
            if i == 0 {
                return None; // 首个页签是账号主页面，关它走 close
            }
            removed = Some(s.tabs.remove(i).external);
            if s.active_tab >= i {
                s.active_tab = s.active_tab.saturating_sub(1).min(s.tabs.len() - 1);
            }
            break;
        }
    }
    refresh_layout(app);
    removed
}

/// 在某个账号下开一个新页签（由 `on_new_window` 调用，也可以给"新建页签"按钮用）。
/// 页面没开着就什么都不做——新页签必须挂在一个已有会话上，才能共享它的 data store。
pub fn open_tab(app: &AppHandle, account_id: &str, url: tauri::Url) -> Result<(), String> {
    let seq = {
        let sessions = app.state::<Sessions>();
        let mut map = sessions.map.lock().map_err(|e| e.to_string())?;
        let s = map.get_mut(account_id).ok_or("这个账号的页面没开着")?;
        // 页签是页面自己 window.open 出来的，数量不由用户控制：写坏的站能一口气开几十个，
        // 每个都是一个真 webview。给个上限，超了就不再开。
        // ponytail: 硬上限够用，真要更细的策略（比如提示用户"这页在疯狂开窗"）再说
        if s.tabs.len() >= MAX_TABS {
            return Err(format!("这个账号已经开了 {MAX_TABS} 个页签，先关掉几个"));
        }
        let seq = s.next_seq;
        s.next_seq += 1;
        seq
    };
    // 新页签不带 cookie 快照也不自动填充：store 是共享的，登录态已经在里面
    let tab = build_tab(app, account_id, seq, url, false, &[])?;
    {
        let sessions = app.state::<Sessions>();
        let mut map = sessions.map.lock().map_err(|e| e.to_string())?;
        let s = map.get_mut(account_id).ok_or("这个账号的页面没开着")?;
        let mut tab = tab;
        tab.return_to = Some(s.active().label.clone());
        s.tabs.push(tab);
        s.active_tab = s.tabs.len() - 1;
    }
    refresh_layout(app);
    Ok(())
}

/// 用户手动给的地址。从聊天里复制的链接常常不带协议头，补上 https；
/// 只放行 http(s)：javascript: / file: 这种进了带登录态的页签就是另一回事了
fn parse_user_url(raw: &str) -> Result<tauri::Url, String> {
    let raw = raw.trim();
    let full = if raw.contains("://") { raw.to_string() } else { format!("https://{raw}") };
    let url: tauri::Url = full.parse().map_err(|_| format!("地址不合法：{raw}"))?;
    match url.scheme() {
        // 带用户名的不要：补协议头后 "mailto:a@b.c" 会被读成用户 mailto 登录 b.c
        "http" | "https" if url.host().is_some() && url.username().is_empty() => Ok(url),
        _ => Err(format!("只能打开 http / https 地址：{raw}")),
    }
}

/// 在账号下开一个新页签打开指定地址。登录态是整个账号共享的，
/// 别人发来的"要登录才能看"的链接在这里开就直接能看
pub fn open_url(app: &AppHandle, account_id: &str, raw: &str) -> Result<(), String> {
    let url = parse_user_url(raw)?;
    {
        let sessions = app.state::<Sessions>();
        let map = sessions.map.lock().map_err(|e| e.to_string())?;
        // 被区域拦截切断的账号再开新页签，等于绕过切断往外发请求
        if map.get(account_id).ok_or("这个账号的页面没开着")?.tabs.iter().any(|t| t.cut) {
            return Err(CUT_MSG.into());
        }
    }
    open_tab(app, account_id, url)
}

/// 切到某个页签。
pub fn select_tab(app: &AppHandle, account_id: &str, index: usize) -> Result<(), String> {
    let popup = {
        let sessions = app.state::<Sessions>();
        let mut map = sessions.map.lock().map_err(|e| e.to_string())?;
        let s = map.get_mut(account_id).ok_or("这个账号的页面没开着")?;
        let t = s.tabs.get(index).ok_or("没有这个页签")?;
        // 弹窗有自己的窗框，"切过去"就是把那个窗口叫到前面来；
        // active_tab 不动——主窗口那块地方还是原来那个页签占着
        if t.external {
            Some(t.label.clone())
        } else {
            s.active_tab = index;
            None
        }
    };
    if let Some(label) = popup {
        if let Some(w) = app.get_webview_window(&label) {
            let _ = w.set_focus();
        }
        return Ok(());
    }
    refresh_layout(app);
    Ok(())
}

/// 关掉某个页签。首个页签不给关——关掉等于关账号，那件事走侧栏的 ×，
/// 而且 `label_for(account_id)` 那一堆调用方都指着它。
pub fn close_tab(app: &AppHandle, account_id: &str, index: usize) -> Result<(), String> {
    if index == 0 {
        return Err("这是账号的主页面，要关请用左边账号上的 ×".into());
    }
    let (label, external) = {
        let sessions = app.state::<Sessions>();
        let mut map = sessions.map.lock().map_err(|e| e.to_string())?;
        let s = map.get_mut(account_id).ok_or("这个账号的页面没开着")?;
        if index >= s.tabs.len() {
            return Err("没有这个页签".into());
        }
        let t = s.tabs.remove(index);
        // 关掉的是当前页签或它左边的，active 都要往左退一格
        if s.active_tab >= index {
            s.active_tab = s.active_tab.saturating_sub(1).min(s.tabs.len() - 1);
        }
        (t.label, t.external)
    };
    close_view(app, &label, external)?;
    refresh_layout(app);
    Ok(())
}

/// 关掉一个页签背后的东西。弹窗要关**窗口**——只关 webview 会剩下一个空窗框。
fn close_view(app: &AppHandle, label: &str, external: bool) -> Result<(), String> {
    if external {
        if let Some(w) = app.get_webview_window(label) {
            w.close().map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    if let Some(v) = app.get_webview(label) {
        v.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 已经开着就切过去；没开就在主窗口里挂一个新的账号 webview。
/// `cookies` 是上次退出时抄下来的快照，由调用方从加密库里取。
pub fn open_or_focus(
    app: &AppHandle,
    account: &Account,
    cookies: &[String],
) -> Result<(), String> {
    let label = label_for(&account.id);
    let sessions = app.state::<Sessions>();

    let exists = sessions
        .map
        .lock()
        .map_err(|e| e.to_string())?
        .contains_key(&account.id)
        && app.get_webview(&label).is_some();

    if !exists {
        let url = account.login_url.trim();
        if url.is_empty() {
            return Err("这个账号还没填登录直达 URL".into());
        }
        let parsed: tauri::Url = url.parse().map_err(|_| format!("登录 URL 不合法：{url}"))?;

        // 只有首个页签自动填充：新页签共享登录态，用不着再填
        let tab = build_tab(app, &account.id, 0, parsed, true, cookies)?;

        sessions.map.lock().map_err(|e| e.to_string())?.insert(
            account.id.clone(),
            SessionState {
                tabs: vec![tab],
                active_tab: 0,
                next_seq: 1,
                platform: account.platform.clone(),
                related_app: if account.related_app.is_empty() {
                    account.username.clone()
                } else {
                    account.related_app.clone()
                },
                opened_at: Instant::now(),
            },
        );
    }

    *sessions.active.lock().map_err(|e| e.to_string())? = account.id.clone();
    *sessions.visible.lock().map_err(|e| e.to_string())? = true;
    refresh_layout(app);
    Ok(())
}

/// 这个账号的页面是不是已经开着。
/// 区域检查要用：已经打开的页面不做二次拦截——网络中途变了就把用户
/// 正在操作的页面锁死，破坏性大于收益。只在**新建**页面时判。
pub fn is_open(app: &AppHandle, account_id: &str) -> bool {
    let sessions = app.state::<Sessions>();
    sessions
        .map
        .lock()
        .map(|m| m.contains_key(account_id))
        .unwrap_or(false)
        && app.get_webview(&label_for(account_id)).is_some()
}

pub fn list(app: &AppHandle) -> Result<Vec<SessionInfo>, String> {
    let sessions = app.state::<Sessions>();
    let map = sessions.map.lock().map_err(|e| e.to_string())?;
    let mut out: Vec<SessionInfo> = map
        .iter()
        .map(|(id, s)| {
            let t = s.active();
            SessionInfo {
                account_id: id.clone(),
                window_label: t.label.clone(),
                platform: s.platform.clone(),
                related_app: s.related_app.clone(),
                opened_secs: s.opened_at.elapsed().as_secs(),
                loaded: t.loaded.load(Ordering::Relaxed),
                failed: t.failed(),
                fail_reason: t.failure(),
                cut: t.cut,
                tabs: s
                    .tabs
                    .iter()
                    .map(|t| TabInfo {
                        title: t.title.lock().map(|x| x.clone()).unwrap_or_default(),
                        url: t.last_url.lock().map(|x| x.clone()).unwrap_or_default(),
                        loaded: t.loaded.load(Ordering::Relaxed),
                        failed: t.failed(),
                        external: t.external,
                    })
                    .collect(),
                active_tab: s.active_tab,
            }
        })
        .collect();
    drop(map);
    // 顺序固定：先开的排上面
    out.sort_by(|a, b| b.opened_secs.cmp(&a.opened_secs));
    apply_layout(app, false);
    Ok(out)
}

/// 当前显示的是哪个账号（前端高亮用）
pub fn active_account(app: &AppHandle) -> String {
    app.state::<Sessions>()
        .active
        .lock()
        .map(|v| v.clone())
        .unwrap_or_default()
}

pub fn close(app: &AppHandle, account_id: &str) -> Result<(), String> {
    let views: Vec<(String, bool)> = {
        let sessions = app.state::<Sessions>();
        let mut map = sessions.map.lock().map_err(|e| e.to_string())?;
        match map.remove(account_id) {
            Some(s) => s.tabs.into_iter().map(|t| (t.label, t.external)).collect(),
            None => return Ok(()),
        }
    };
    for (label, external) in views {
        close_view(app, &label, external)?;
    }

    // 关掉的正好是当前显示的那个，就自动切到还开着的第一个
    let sessions = app.state::<Sessions>();
    let is_active = sessions.active.lock().map(|a| *a == account_id).unwrap_or(false);
    if is_active {
        let next = list(app)?.first().map(|s| s.account_id.clone()).unwrap_or_default();
        *sessions.active.lock().map_err(|e| e.to_string())? = next;
    }
    refresh_layout(app);
    Ok(())
}

pub fn close_all(app: &AppHandle) -> Result<(), String> {
    let views: Vec<(String, bool)> = {
        let sessions = app.state::<Sessions>();
        let mut map = sessions.map.lock().map_err(|e| e.to_string())?;
        map.drain()
            .flat_map(|(_, s)| s.tabs.into_iter().map(|t| (t.label, t.external)))
            .collect()
    };
    for (label, external) in views {
        let _ = close_view(app, &label, external);
    }
    let sessions = app.state::<Sessions>();
    *sessions.active.lock().map_err(|e| e.to_string())? = String::new();
    *sessions.visible.lock().map_err(|e| e.to_string())? = false;
    Ok(())
}

/// 账号在磁盘上的登录数据（cookie、LocalStorage、IndexedDB）放在哪：
/// - macOS：WebKit 按 data store identifier 放在 `~/Library/WebKit/<bundle id>/WebsiteDataStore/<uuid>`，
///   **根本不在我们的数据目录里**；开发版没打包成 .app，`<bundle id>` 那一级是可执行文件名
/// - Windows / Linux：`profile_dir`（macOS 上它是空目录，一并删掉）
fn store_dirs(app: &AppHandle, account_id: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(d) = crate::data_dir(app) {
        out.push(d.join("profiles").join(account_id));
    }
    #[cfg(target_os = "macos")]
    if let Ok(home) = app.path().home_dir() {
        let exe = std::env::current_exe().ok();
        let bundled = exe.as_ref().is_some_and(|p| p.to_string_lossy().contains(".app/Contents/MacOS/"));
        let owner = if bundled {
            Some(app.config().identifier.clone())
        } else {
            exe.and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        };
        if let Some(owner) = owner {
            out.push(home.join("Library/WebKit").join(owner).join("WebsiteDataStore").join(store_dir_name(account_id)));
        }
    }
    out
}

/// WebKit 给这个 data store 建的目录名：identifier 那 16 个字节写成小写 uuid（只 macOS 用得上）
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn store_dir_name(account_id: &str) -> String {
    let hex: String = store_identifier(account_id).iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

/// 账号 id 要拼进路径去删目录，只认 uuid 那几种字符，`..` 之类的一律不碰
fn safe_id(account_id: &str) -> bool {
    !account_id.is_empty() && account_id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// 抹掉这些账号留在磁盘上的登录数据，并关掉它们开着的页面。
///
/// 开着的页面先让 WebKit 自己把 data store 清空再关：关掉之后它的网络进程可能还把 cookie
/// 攥在内存里，只删目录的话会被写回来。关着的账号直接删目录。
/// cookie 快照在加密库里另有一份（调用方先抄），下次打开照样塞回去。
pub fn purge(app: &AppHandle, account_ids: &[String]) {
    let open: Vec<String> = {
        let sessions = app.state::<Sessions>();
        let Ok(map) = sessions.map.lock() else { return };
        map.keys().filter(|id| account_ids.contains(id)).cloned().collect()
    };
    for id in &open {
        // cookie 这些是 data store 级的，同一账号的页签、弹窗共用一份，清首个页签的就够
        if let Ok(v) = first_webview(app, id) {
            let _ = v.clear_all_browsing_data();
        }
        let _ = close(app, id);
    }
    for id in account_ids.iter().filter(|id| safe_id(id)) {
        for dir in store_dirs(app, id) {
            if std::fs::remove_dir_all(&dir).is_err() && dir.exists() {
                retry_remove(app, id, dir);
            }
        }
    }
}

/// WebView2 关掉之后，它的浏览器进程还会占着数据目录几秒，这时候删会失败（macOS 的 WebKit 没这个问题，
/// 第一次就删掉了，走不到这里）。后台隔一会儿再删几次；账号要是又被打开了就停手——
/// 删一个正在用的数据目录，删掉一半只会把它弄坏
fn retry_remove(app: &AppHandle, account_id: &str, dir: PathBuf) {
    let (app, id) = (app.clone(), account_id.to_string());
    std::thread::spawn(move || {
        for secs in [1, 2, 3, 5, 10] {
            std::thread::sleep(std::time::Duration::from_secs(secs));
            if is_open(&app, &id) {
                return;
            }
            if std::fs::remove_dir_all(&dir).is_ok() || !dir.exists() {
                return;
            }
        }
        eprintln!("[switchboard] 登录数据目录一直被占用，没删掉：{}", dir.display());
    });
}

/// 这个账号**当前页签**的 webview。填充 / 指认 / 取地址 / 刷新都作用在它身上。
fn webview_of(app: &AppHandle, account_id: &str) -> Result<Webview, String> {
    let label = app
        .state::<Sessions>()
        .map
        .lock()
        .map_err(|e| e.to_string())?
        .get(account_id)
        .map(|s| s.active().label.clone())
        .ok_or("这个账号的页面没开着")?;
    app.get_webview(&label).ok_or_else(|| "页面已丢失".to_string())
}

/// 首个页签的 webview。只有 cookie 快照用它——store 级的东西跟页签无关。
fn first_webview(app: &AppHandle, account_id: &str) -> Result<Webview, String> {
    app.get_webview(&label_for(account_id))
        .ok_or_else(|| "这个账号的页面没开着".to_string())
}

const CUT_MSG: &str = "网络被区域拦截切断了，页面已断开；网络恢复后会自动接回";

/// 前进 / 后退。被切断的页面停在 about:blank，一后退就回到原来那页、请求又发出去了，所以要挡
pub fn go_history(app: &AppHandle, account_id: &str, delta: i32) -> Result<(), String> {
    let cut = {
        let sessions = app.state::<Sessions>();
        let map = sessions.map.lock().map_err(|e| e.to_string())?;
        map.get(account_id).ok_or("这个账号的页面没开着")?.active().cut
    };
    if cut {
        return Err(CUT_MSG.into());
    }
    eval_active(app, account_id, &format!("history.go({delta});"))
}

/// 往这个账号当前页签里跑一段脚本。指认写回配置后立刻试填一次用得上。
pub fn eval_active(app: &AppHandle, account_id: &str, js: &str) -> Result<(), String> {
    webview_of(app, account_id)?.eval(js).map_err(|e| e.to_string())
}

/// 页面打不开时，往里注脚本是**静默无效**的（没有输入框可填、没有元素可点）。
/// 与其让用户点了按钮一脸懵，不如直说。
fn ensure_usable(app: &AppHandle, account_id: &str) -> Result<(), String> {
    let sessions = app.state::<Sessions>();
    let map = sessions.map.lock().map_err(|e| e.to_string())?;
    let t = map.get(account_id).ok_or("这个账号的页面没开着")?.active();
    if t.cut {
        return Err(CUT_MSG.into());
    }
    match t.failure() {
        Some(r) => Err(format!("页面没打开（{r}），先点「刷新重试」")),
        None if !t.loaded.load(Ordering::Relaxed) => Err("页面还在加载，等它出来再操作".into()),
        None => Ok(()),
    }
}

/// 把一个值填进用户最后点过的那个输入框——不依赖任何选择器的兜底手段。
/// `login` 给了就先核站点：密码这类值只往账号登录地址所在的站填。
/// 页签是会跳走的（点了个链接、在账号里开了别的地址），用户以为还在登录页，一点「填密码」就填给了别人。
/// 只认域名 / IP，见 `same_host`
pub fn fill_focused(app: &AppHandle, account_id: &str, value: &str, what: &str, login: Option<&str>) -> Result<(), String> {
    ensure_usable(app, account_id)?;
    if let Some(login) = login {
        let page = {
            let sessions = app.state::<Sessions>();
            let map = sessions.map.lock().map_err(|e| e.to_string())?;
            let t = map.get(account_id).ok_or("这个账号的页面没开着")?.active();
            let page = t.last_url.lock().map(|u| u.clone()).unwrap_or_default();
            page
        };
        let (Ok(l), Ok(p)) = (login.trim().parse::<tauri::Url>(), page.parse::<tauri::Url>()) else {
            return Err(format!("认不出当前页面在哪个站，{what}先不填"));
        };
        if !same_host(&l, &p) {
            return Err(format!(
                "当前页面在 {}，跟这个账号登录地址的域名 / IP（{}）对不上，{what}不往别的站填。确实要在这里登录，先点「设为直达页」",
                p.host_str().unwrap_or("?"),
                l.host_str().unwrap_or("?"),
            ));
        }
    }
    webview_of(app, account_id)?
        .eval(&adapter::fill_focused_js(value, what))
        .map_err(|e| e.to_string())
}

/// 在当前页签里打开查找框。原生 webview 得先拿到焦点，不然查找框里打不了字
pub fn find_in_page(app: &AppHandle, account_id: &str) -> Result<(), String> {
    ensure_usable(app, account_id)?;
    let v = webview_of(app, account_id)?;
    v.set_focus().map_err(|e| e.to_string())?;
    v.eval(adapter::find_js()).map_err(|e| e.to_string())
}

/// 账号页面当前停在哪个地址（用来"把当前页设为登录直达 URL"）。
///
/// 页面没加载成功时**绝不能**去问地址——见 `SessionState::url_is_safe`，
/// 那会让整个进程 SIGABRT。白板上点「设为直达页」正好踩的就是这条。
pub fn current_url(app: &AppHandle, account_id: &str) -> Result<String, String> {
    {
        let sessions = app.state::<Sessions>();
        let map = sessions.map.lock().map_err(|e| e.to_string())?;
        let s = map.get(account_id).ok_or("这个账号的页面没开着")?.active();
        if !s.url_is_safe() {
            return Err("页面还没加载出来，没有地址可取。先点「刷新重试」把页面打开".into());
        }
    }
    webview_of(app, account_id)?
        .url()
        .map(|u| u.to_string())
        .map_err(|e| e.to_string())
}

/// 把所有账号页面**切断**：全部导航到 `about:blank`。
///
/// 为什么是导航而不是藏起来：藏只是不显示，页面照样在后台轮询、发心跳、传数据——
/// "拦截"要的是**真的一个请求都出不去**。导航走之后整个文档被销毁，
/// 连带它所有在飞的请求、定时器、WebSocket 全断。
///
/// 为什么不直接 close：close 掉再开是全新的 webview，正在填的表单、
/// 没提交的东西全没。切到空白页则保留 webview 和它的 cookie 仓，恢复时导航回去就行。
///
/// 返回被切断的账号数。调用方**必须先抄一次 cookie**——导航会丢掉内存里的会话态。
pub fn cut_all(app: &AppHandle) -> usize {
    let sessions = app.state::<Sessions>();
    let Ok(mut map) = sessions.map.lock() else { return 0 };
    let mut n = 0;
    // 一个账号的每个页签都要切：漏一个就等于那一页还在往外发请求
    for t in map.values_mut().flat_map(|s| s.tabs.iter_mut()) {
        if t.cut {
            continue;
        }
        let Some(v) = app.get_webview(&t.label) else { continue };
        let Ok(blank) = "about:blank".parse() else { continue };
        if v.navigate(blank).is_ok() {
            t.cut = true;
            t.loaded.store(false, Ordering::Relaxed);
            t.committed.store(false, Ordering::Relaxed);
            n += 1;
        }
    }
    drop(map);
    refresh_layout(app);
    n
}

/// 解除切断：每个页面导航回它被切断前停的那个地址。
/// cookie 仓没动过，所以登录态还在，不用重新登。
pub fn restore_all(app: &AppHandle) -> usize {
    let sessions = app.state::<Sessions>();
    let Ok(mut map) = sessions.map.lock() else { return 0 };
    let mut n = 0;
    for t in map.values_mut().flat_map(|s| s.tabs.iter_mut()) {
        if !t.cut {
            continue;
        }
        let Some(v) = app.get_webview(&t.label) else { continue };
        let url = t.last_url.lock().map(|u| u.clone()).unwrap_or_default();
        let Ok(parsed) = url.parse() else { continue };
        if v.navigate(parsed).is_ok() {
            t.cut = false;
            t.fail = None;
            t.nav_at = Instant::now(); // 重新开始算加载超时
            // 弹窗没挂 on_page_load，没人会把它标回"已加载"；不补这一下，
            // 切断再接回之后 12 秒就被加载超时判成"打不开"
            if t.external {
                t.loaded.store(true, Ordering::Relaxed);
            }
            n += 1;
        }
    }
    drop(map);
    refresh_layout(app);
    n
}

/// 当前有没有页面处于被切断状态
pub fn any_cut(app: &AppHandle) -> bool {
    app.state::<Sessions>()
        .map
        .lock()
        .map(|m| m.values().flat_map(|s| s.tabs.iter()).any(|t| t.cut))
        .unwrap_or(false)
}

/// 页面自报打不开（HTTP 4xx/5xx 或纯白板）。由 `report_page_state` 命令调用。
pub fn mark_failure(app: &AppHandle, label: &str, reason: Option<String>) {
    {
        let sessions = app.state::<Sessions>();
        let Ok(mut map) = sessions.map.lock() else { return };
        let Some(t) = map.values_mut().find_map(|s| s.tab_of(label)) else { return };
        if t.fail == reason {
            return; // 没变化就别白折腾一次重排
        }
        t.fail = reason;
    }
    refresh_layout(app);
}

/// 重新加载这个账号的页面。加载失败后的「刷新」按钮走这里。
pub fn reload(app: &AppHandle, account_id: &str, url: &str) -> Result<(), String> {
    let v = webview_of(app, account_id)?;
    // 刷新回页面自己停的地址，否则在控制台里点一下刷新就被踹回登录页，正在看的那一页就丢了。
    // 只有首个页签打不开时才回账号的登录直达 URL：那多半是地址本身有问题，用户可能刚改过
    let target = {
        let sessions = app.state::<Sessions>();
        let mut map = sessions.map.lock().map_err(|e| e.to_string())?;
        let s = map.get_mut(account_id).ok_or("这个账号的页面没开着")?;
        // 被切断的页面刷新就等于接回去了：页面藏着，请求却照样往外发，区域拦截白设
        if s.active().cut {
            return Err(CUT_MSG.into());
        }
        let own = if s.active_tab == 0 && s.active().failed() {
            String::new()
        } else {
            s.active().last_url.lock().map(|u| u.clone()).unwrap_or_default()
        };
        let t = s.active_mut();
        if let Ok(mut armed) = t.fill_armed.lock() {
            *armed = Instant::now(); // 登录过期后点刷新，自动填充得重新管用
        }
        t.loaded.store(false, Ordering::Relaxed);
        t.committed.store(false, Ordering::Relaxed);
        t.fail = None; // 上次的失败结论作废，重新来过
        t.nav_at = Instant::now(); // 重新开始算超时
        if own.trim().is_empty() { url.trim().to_string() } else { own }
    };
    let parsed = target.parse().map_err(|_| format!("地址不合法：{target}"))?;
    v.navigate(parsed).map_err(|e| e.to_string())?;
    refresh_layout(app);
    Ok(())
}

/// 进入/退出"手动指认输入框"模式。
pub fn set_picking(app: &AppHandle, account_id: &str, on: bool) -> Result<(), String> {
    if on {
        ensure_usable(app, account_id)?;
    }
    {
        let sessions = app.state::<Sessions>();
        let mut map = sessions.map.lock().map_err(|e| e.to_string())?;
        let s = map.get_mut(account_id).ok_or("这个账号的页面没开着")?;
        // 退出指认时把**所有**页签的标记都清掉：用户可能在指认途中切了页签，
        // 只清当前那个会留下一个永远处在指认模式的页签，之后它点谁都当指认
        if !on {
            for t in s.tabs.iter_mut() {
                t.picking = false;
            }
        } else {
            s.active_mut().picking = true;
        }
    }
    if on {
        return webview_of(app, account_id)?
            .eval(&adapter::picker_js())
            .map_err(|e| e.to_string());
    }
    // 关的时候也要挨个关：指认途中切过页签的话，蓝条和取点脚本会留在别的页签上
    let labels: Vec<String> = {
        let sessions = app.state::<Sessions>();
        let map = sessions.map.lock().map_err(|e| e.to_string())?;
        map.get(account_id)
            .map(|s| s.tabs.iter().map(|t| t.label.clone()).collect())
            .unwrap_or_default()
    };
    for label in labels {
        if let Some(v) = app.get_webview(&label) {
            let _ = v.eval(adapter::stop_picker_js());
        }
    }
    Ok(())
}

/// 页面回传选择器时用：找出哪个账号正在指认模式。
pub fn picking_account(app: &AppHandle, label: &str) -> Option<String> {
    let sessions = app.state::<Sessions>();
    let map = sessions.map.lock().ok()?;
    map.iter()
        .find(|(_, s)| s.tabs.iter().any(|t| t.picking && t.label == label))
        .map(|(id, _)| id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 首个页签的 label **不能**变个样：`report_picked_selectors` 靠 label 找账号，
    /// capability 的 `acct-*` 也指着它；后开的页签只在后面接 `-t<n>`，仍然命中 `acct-*`。
    #[test]
    fn tab_labels_keep_the_first_tab_compatible() {
        let id = "9f8e7d6c-1111-2222-3333-444455556666";
        assert_eq!(tab_label(id, 0), label_for(id), "首个页签必须和 label_for 完全一致");
        assert_eq!(tab_label(id, 1), format!("{}-t1", label_for(id)));
        assert!(tab_label(id, 7).starts_with("acct-"), "新页签也要命中 capability 的 acct-*");
        // 非法字符要被换成 '-'，否则 wry 那边的 label 不合法
        assert_eq!(tab_label("a b/c", 2), "acct-a-b-c-t2");
    }

    /// 下载文件名是**远端给的**，直接 join 进「下载」目录是有风险的：
    /// `..` 能把落点挪到上一级。这里把几种脏输入钉住。
    #[test]
    fn download_names_cannot_escape_the_folder() {
        let name = |u: &str| download_name("", &u.parse::<tauri::Url>().unwrap());
        assert_eq!(name("https://x.com/a/b/证书.zip"), "证书.zip");
        assert_eq!(name("https://x.com/a/.."), "download", "..不能当文件名");
        assert_eq!(name("https://x.com/a/"), "download", "空段退回默认名");
        assert_eq!(name("https://x.com/"), "download");
        // 百分号编码的斜杠不会被 Url 解码成真斜杠，但真斜杠/冒号也得挡住
        assert_eq!(name("https://x.com/a%2Fb.pdf"), "a-b.pdf", "解码出来的斜杠要被清掉");
        for n in [name("https://x.com/a/..%2F..%2Fetc%2Fpasswd"), name("https://x.com/a/x.pdf")] {
            assert!(!n.contains('/') && n != ".." && n != ".", "脏名字漏过去了：{n}");
        }
        // 建议名优先；blob: 地址自己没有文件名
        let blob: tauri::Url = "blob:https://x.com/3f2a".parse().unwrap();
        assert_eq!(download_name("报表.xlsx", &blob), "报表.xlsx");
        assert_eq!(download_name("", &blob), "download");
        assert_eq!(download_name("../../etc/passwd", &blob), "-..-etc-passwd", "建议名一样要清洗");
        assert_eq!(download_name("..", &"https://x.com/a.pdf".parse().unwrap()), "a.pdf", "建议名不像样就退回地址");
    }

    /// Windows 的额外清洗。macOS 那条路（windows=false）必须跟以前一模一样
    #[test]
    fn download_names_follow_windows_rules() {
        assert_eq!(clean_name("报表<1>?.xlsx", true), "报表-1--.xlsx", "Windows 建不了的字符要换掉");
        assert_eq!(clean_name("a\"b|c*.txt", true), "a-b-c-.txt");
        assert_eq!(clean_name("nul.txt", true), "_nul.txt", "设备名写进去就没了");
        assert_eq!(clean_name("COM1", true), "_COM1");
        assert_eq!(clean_name("console.log", true), "console.log", "只是开头像不算");
        assert_eq!(clean_name("x.pdf. .", true), "x.pdf", "末尾的点和空格系统会悄悄去掉，先去掉");
        assert_eq!(clean_name("报表<1>?.xlsx", false), "报表<1>?.xlsx", "macOS 上这些字符是合法的，别动");
        assert_eq!(clean_name("nul.txt", false), "nul.txt");
    }

    /// 新页签的 label 必须仍然落在 capability 的 `webviews` 模式里，否则它的
    /// `report_page_state` / `report_picked_selectors` 会被 ACL 静默拒掉 ——
    /// 就是 `report_picked_selectors not allowed` 那次的坑，换个壳再来一遍。
    /// 匹配用的就是 tauri 自己那套 `glob::Pattern`（tauri-utils acl/resolved.rs）。
    #[test]
    fn every_tab_label_is_covered_by_the_session_capability() {
        let cap: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/session.json")).unwrap();
        let pats: Vec<glob::Pattern> = cap["webviews"]
            .as_array()
            .expect("session capability 必须声明 webviews")
            .iter()
            .map(|p| glob::Pattern::new(p.as_str().unwrap()).unwrap())
            .collect();
        let id = "9f8e7d6c-1111-2222-3333-444455556666";
        let labels: Vec<String> = [0u32, 1, 12]
            .iter()
            .map(|s| tab_label(id, *s))
            // 弹窗是**独立窗口**里的 webview，窗口 label 不匹配 `windows: ["main"]`，
            // 全靠 webview label 命中 `acct-*`，所以这条尤其要钉
            .chain([popup_label(id, 3)])
            .collect();
        for label in labels {
            assert!(
                pats.iter().any(|p| p.matches(&label)),
                "页签 label {label} 不在 capability 的 webviews 里，它的回传会被 ACL 拒掉",
            );
        }
    }

    /// 页签条的高度前后端各存一份（跟 TOPBAR_H 一样的既有约定）。
    /// 对不上的后果是页面和页签条之间露一条缝、或者页面盖住页签条，
    /// 而且只能靠肉眼看出来——所以用源码对一遍。
    #[test]
    fn frontend_layout_constants_match() {
        let ts = include_str!("../../src/screens/Layout.tsx");
        for (name, value) in [("TOPBAR_H", TOPBAR_H), ("TABS_H", TABS_H)] {
            let want = format!("{name} = {}", value as i64);
            assert!(
                ts.contains(&want),
                "src/screens/Layout.tsx 里没有 `{want}`，前后端的 {name} 对不上了",
            );
        }
    }

    /// cookie 快照是"读出来转字符串存库 → 下次解析回来塞进去"。
    /// 只要 Display/parse 这一圈丢了 domain 或过期时间，恢复就等于没做，
    /// 而且是**静默**失效——页面照样打开，只是还要重新登录。所以钉死这一圈。
    #[test]
    fn cookie_survives_the_snapshot_round_trip() {
        use tauri::webview::cookie::time::{Duration, OffsetDateTime};
        use tauri::webview::cookie::{Cookie, Expiration};

        let expires = OffsetDateTime::now_utc() + Duration::days(30);
        let mut c = Cookie::new("skey", "abc123");
        c.set_domain(".cloud.tencent.com");
        c.set_path("/");
        c.set_secure(true);
        c.set_http_only(true);
        c.set_expires(Expiration::DateTime(expires));

        // 存库时就是这么序列化的
        let raw = c.to_string();
        let back = Cookie::parse(raw).expect("存进去的字符串必须还能解析回来");

        assert_eq!(back.name(), "skey");
        assert_eq!(back.value(), "abc123");
        assert_eq!(back.domain(), Some("cloud.tencent.com"), "domain 丢了的话 cookie 会挂错域名");
        assert_eq!(back.path(), Some("/"));
        assert_eq!(back.secure(), Some(true));
        assert_eq!(back.http_only(), Some(true), "登录态基本都是 HttpOnly，丢了就白做");
        assert!(back.expires_datetime().is_some(), "过期时间丢了会退化成会话 cookie");
    }

    /// 会话 cookie（不带过期时间）也要能原样round-trip：它正是"关了 App 就掉登录"的主角，
    /// 抄快照的意义就在这里。
    #[test]
    fn session_cookie_round_trips_without_expiry() {
        use tauri::webview::cookie::Cookie;

        let mut c = Cookie::new("PHPSESSID", "zzz");
        c.set_domain("example.com");
        c.set_path("/");
        let back = Cookie::parse(c.to_string()).unwrap();
        assert_eq!(back.name(), "PHPSESSID");
        assert_eq!(back.domain(), Some("example.com"));
        assert!(back.expires_datetime().is_none());
    }

    fn tab(label: &str, seen: bool, external: bool, return_to: Option<&str>) -> Tab {
        Tab {
            label: label.into(),
            nav_at: Instant::now() - std::time::Duration::from_secs(60), // 早就超时了
            loaded: Arc::new(AtomicBool::new(seen)),
            committed: Arc::new(AtomicBool::new(seen)),
            fail: None,
            last_url: Arc::new(Mutex::new(String::new())),
            title: Arc::new(Mutex::new(String::new())),
            fill_armed: Arc::new(Mutex::new(Instant::now())),
            cut: false,
            shown: false,
            picking: false,
            external,
            seen_page: Arc::new(AtomicBool::new(seen)),
            download_only: false,
            return_to: return_to.map(Into::into),
        }
    }

    #[test]
    fn user_url_gets_https_and_only_http_passes() {
        assert_eq!(parse_user_url(" help.aliyun.com/doc?id=1 ").unwrap().as_str(), "https://help.aliyun.com/doc?id=1");
        assert_eq!(parse_user_url("http://203.0.113.10:8888/x").unwrap().as_str(), "http://203.0.113.10:8888/x");
        assert_eq!(parse_user_url("localhost:8080/x").unwrap().as_str(), "https://localhost:8080/x");
        for bad in ["javascript:alert(1)", "file:///etc/passwd", "mailto:a@b.c", "", "   "] {
            assert!(parse_user_url(bad).is_err(), "{bad} 不该放行");
        }
    }

    /// 宝塔面板那个 1MB 的 CSS 在小带宽服务器上要下 16 秒：服务器早就回话了，只是 Finished 来得晚。
    /// 这种不能判成"打不开"；一个字节都没回来的才算
    #[test]
    fn slow_page_that_answered_is_not_a_timeout() {
        let t = tab("t", false, false, None);
        assert!(t.failed(), "没回话 + 超时 = 打不开");
        t.committed.store(true, Ordering::Relaxed);
        assert!(!t.failed(), "回过话就只是慢，接着等");
    }

    /// 页面 window.open / target=_blank 一个下载地址：开出来的新页签里什么都没有。
    /// 不处理的话 12 秒后被判成"打不开"，整块区域变成错误提示——用户说的"一下载页面就崩"
    #[test]
    fn download_only_tab_sends_user_back_and_never_fails() {
        let mut s = SessionState {
            tabs: vec![tab("t0", true, false, None), tab("t1", true, false, Some("t0")), tab("t2", false, false, Some("t1"))],
            active_tab: 2,
            next_seq: 3,
            platform: String::new(),
            related_app: String::new(),
            opened_at: Instant::now(),
        };
        assert!(s.tabs[2].failed(), "不处理的话：没加载过的新页签超时就判成打不开");
        assert!(s.mark_download_only("t2"));
        assert_eq!(s.active_tab, 1, "送回开它的那个页签，不是最左边");
        assert!(!s.tabs[2].failed(), "下载专用页签不算打不开");
        assert_eq!(*s.tabs[2].title.lock().unwrap(), "下载中…");
        assert!(s.mark_download_only("t2"), "同一个页签再来一次下载也认");

        assert!(!s.mark_download_only("t1"), "显示过页面的页签里点下载，页签留着");
        assert!(!s.mark_download_only("t0"), "首个页签是账号主页面，永远不关");
        s.tabs[0].seen_page.store(false, Ordering::Relaxed);
        assert!(!s.mark_download_only("t0"), "哪怕它还没加载出来");

        // 用户已经切去别的页签了，就别动 active；开它的页签没了就回首个页签
        let mut s2 = SessionState { tabs: vec![tab("a", true, false, None), tab("b", false, false, Some("gone"))], active_tab: 0, ..s };
        assert!(s2.mark_download_only("b"));
        assert_eq!(s2.active_tab, 0);
        s2.active_tab = 1;
        s2.tabs[1].download_only = false;
        assert!(s2.mark_download_only("b"));
        assert_eq!(s2.active_tab, 0, "回去的页签不在了就回首个页签");
    }

    /// 删登录数据要找对目录：WebKit 用 identifier 的小写 uuid 当目录名，跟账号 id 一模一样。
    /// 这条要是错了，删账号时登录态照样留在磁盘上
    #[test]
    fn store_dir_matches_the_account_uuid() {
        let id = "0e503284-dac3-4f64-9fc1-1bf0d43d124f";
        assert_eq!(store_dir_name(id), id);
        assert!(safe_id(id));
        assert!(!safe_id("../x") && !safe_id("") && !safe_id("a/b"), "能拼出上级目录的 id 不能拿去删");
    }

    #[test]
    fn identifier_is_stable_and_derived_from_the_uuid() {
        let id = "0f8fad5b-d9cb-469f-a165-70867728950e";
        assert_eq!(store_identifier(id), store_identifier(id));
        assert_eq!(store_identifier(id)[0], 0x0f);
        assert_eq!(store_identifier(id)[15], 0x0e);
        assert_ne!(store_identifier(id), store_identifier("7c9e6679-7425-40de-944b-e07fc1f90ae7"));
    }

    /// 带明文密码的填充脚本只能落在登录页那个站上。页签跳到别的站之后还填，
    /// 那边随便一个 `input[type=password]` 或者一句 `window.__sb = ...` 就把密码拿走了
    #[test]
    fn autofill_stays_on_the_login_site() {
        let ok = |a: &str, b: &str| fill_allowed(&a.parse().unwrap(), &b.parse().unwrap());
        assert!(ok("https://signin.aliyun.com/login.htm", "https://signin.aliyun.com/x"));
        assert!(ok("https://account.aliyun.com/login", "https://signin.aliyun.com/login.htm"), "同站跳转要留着");
        assert!(ok("https://a.example.com.cn/", "https://b.example.com.cn/"));
        assert!(!ok("https://signin.aliyun.com/", "https://evil.com/"), "跳到别的站不能再填");
        assert!(!ok("https://a.example.com.cn/", "https://other.com.cn/"), "com.cn 不能当成一个站");
        assert!(!ok("https://a.co.uk/", "https://b.co.uk/"), "co.uk 同理");
        assert!(!ok("https://signin.aliyun.com/", "http://signin.aliyun.com/"), "https 的密码不填进 http 页");
        assert!(ok("http://1.2.3.4:8888/login", "http://1.2.3.4:8888/"));
        assert!(!ok("http://1.2.3.4:8888/login", "http://1.2.3.4:9000/"), "同一台机器上别的服务也不行");
        assert!(!ok("http://localhost:8080/", "http://localhost:9090/"));
        assert!(!ok("https://signin.aliyun.com/", "https://1.2.3.4/"));
    }

    #[test]
    fn manual_fill_only_checks_domain_or_ip() {
        let ok = |a: &str, b: &str| same_host(&a.parse().unwrap(), &b.parse().unwrap());
        assert!(ok("https://signin.aliyun.com/", "https://account.aliyun.com/x"), "同一个域名的子域名");
        assert!(ok("https://signin.aliyun.com/", "http://signin.aliyun.com/"), "协议不管");
        assert!(ok("http://203.0.113.10:8888/login", "http://203.0.113.10:9000/"), "同一个 IP 换端口也行");
        assert!(ok("http://localhost:8080/", "http://localhost:9090/"));
        assert!(!ok("https://signin.aliyun.com/", "https://evil.com/"));
        assert!(!ok("https://a.example.com.cn/", "https://other.com.cn/"), "com.cn 不能当成一个域名");
        assert!(!ok("http://203.0.113.10/", "http://203.0.113.11/"));
        assert!(!ok("https://signin.aliyun.com/", "https://203.0.113.10/"));
    }

    #[test]
    fn label_keeps_uuid_readable_and_drops_junk() {
        assert_eq!(label_for("0f8fad5b-d9cb"), "acct-0f8fad5b-d9cb");
        assert_eq!(label_for("a b/c"), "acct-a-b-c");
    }
}
