mod adapter;
mod db;
mod demo;
mod models;
mod region;
mod session;
mod totp;

use models::{Account, PlatformConfig, SessionInfo};
use region::{Endpoint, Origin, Verdict};
use rusqlite::Connection;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{atomic::{AtomicU64, Ordering}, Mutex};
use tauri::{AppHandle, Emitter, Manager, State, Webview};

/// None = 锁定态（还没输主密码 / 已锁回去）。
#[derive(Default)]
struct Vault(Mutex<Option<Connection>>);

/// 区域检测结果缓存。网络不会一秒一变，10 分钟内复用同一个结论，
/// 免得每点一次账号就对外发三个请求。
#[derive(Default)]
struct RegionCache(Mutex<Option<(Verdict, std::time::Instant)>>);

const REGION_CACHE_SECS: u64 = 600;

/// 用户在拦截框里点了「仍要继续」之后，多久之内哨兵不去断页面、也不再弹框问。
/// 没有这段宽限，block 模式下一回到账号列表（会触发一次复查）就把刚放行的页面断掉，「仍要继续」形同虚设；
/// warn 模式也会紧接着再问一遍"要不要断开"。锁定即作废
const FORCE_GRACE_SECS: u64 = 30 * 60;

/// 最近一次「仍要继续」的时间，见 `FORCE_GRACE_SECS`
#[derive(Default)]
struct RegionGrace(Mutex<Option<std::time::Instant>>);

fn grace_active(app: &AppHandle) -> bool {
    app.state::<RegionGrace>()
        .0
        .lock()
        .map(|g| g.is_some_and(|t| t.elapsed().as_secs() < FORCE_GRACE_SECS))
        .unwrap_or(false)
}
const MODE_KEY: &str = "region_guard_mode";
const ENDPOINTS_KEY: &str = "region_probe_endpoints";
const WATCH_SECS_KEY: &str = "region_watch_secs";
const DEFAULT_WATCH_SECS: u64 = 2 * 60 * 60;
/// 多久没操作就自动锁回登录页。0 = 关闭。判断"要不要锁"这个决定留在前端（App.tsx）——
/// 前端是 App 自己唯一的界面，不是要防着的对手，不用照区域检测那样搬一整套后端常驻线程。
const AUTO_LOCK_SECS_KEY: &str = "auto_lock_secs";
/// 哪些账号开了保活：`{账号 id: 网络异常时也保活}`，没开的不在里面。见 session.rs 的 `keep_alive`
const KEEP_ALIVE_KEY: &str = "keep_alive_accounts";

/// "最近一次操作"的时间戳，主窗口和账号页面共用一份。
///
/// 只在前端记这个时间戳不够：账号页面（会话页右侧）是独立的原生子 webview，
/// 用户在里面填外部登录表单时，那些点击/按键根本不会冒泡到主窗口的 DOM，
/// 主窗口的 JS 完全看不见——所以需要这一份后端共享状态，让 `adapter.rs` 里
/// 注入进账号页面的常驻脚本也能把"刚刚有人在动"报回来。
#[derive(Default)]
struct Activity(Mutex<ActivityClock>);

#[derive(Default)]
struct ActivityClock {
    /// 单调钟：有人往回拨系统时间也不会把空闲时长拨没
    mono: Option<std::time::Instant>,
    /// 墙上钟：macOS 的 `Instant` 是 CLOCK_UPTIME_RAW，**睡眠期间不走**——只用它的话，
    /// 合上盖子睡三个小时再打开，空闲时长还是零，自动锁定在最该生效的时候失效
    wall: Option<std::time::SystemTime>,
    /// 自动锁定时长（秒），0 = 关闭。解锁时从库里读，改设置时同步
    limit: u64,
}

impl ActivityClock {
    fn reset(&mut self, limit: u64) {
        self.limit = limit;
        self.mark();
    }

    fn mark(&mut self) {
        self.mono = Some(std::time::Instant::now());
        self.wall = Some(std::time::SystemTime::now());
    }

    /// 两只钟取大的：睡眠要算进去，往回拨表不能算成"刚动过"
    fn idle_secs(&self) -> u64 {
        let mono = self.mono.map(|t| t.elapsed().as_secs()).unwrap_or(0);
        let wall = self.wall.and_then(|t| t.elapsed().ok()).map(|d| d.as_secs()).unwrap_or(0);
        mono.max(wall)
    }

    fn expired(&self) -> bool {
        self.limit > 0 && self.idle_secs() >= self.limit
    }

    /// 记一次操作；已经超时的不续命，返回 true。见 `touch_activity`
    fn touch(&mut self) -> bool {
        if self.expired() {
            return true;
        }
        self.mark();
        false
    }
}

struct RegionWatch {
    secs: AtomicU64,
    changed: tokio::sync::Notify,
}

impl Default for RegionWatch {
    fn default() -> Self {
        Self { secs: AtomicU64::new(DEFAULT_WATCH_SECS), changed: tokio::sync::Notify::new() }
    }
}

pub const DB_FILE: &str = "switchboard.db";

/// Windows 上从 GUI 程序起控制台程序（powershell、reg）会闪一个黑框，带上这个标志就不弹
#[cfg(windows)]
pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 注册时填的用户名另存一份在加密库外面。它是解锁前唯一能显示的身份信息——
/// 库里那份在密文里，没有主密码根本读不到。只是个名字，不涉及任何凭据。
const OWNER_FILE: &str = "owner.txt";

/// 主密码最短长度。跟 src/api.ts 的 MIN_PASSWORD 保持一致。
/// 不做复杂度校验——这是本机自用的库，强度由用户自己权衡。
pub const MIN_PASSWORD_LEN: usize = 3;

/// 本 App 所有落盘数据的根目录。**开发版单独一份**，不然 `pnpm tauri dev`
/// 调试时写的就是你日常在用的那个真实账号库——改坏了没得后悔。
/// 会话隔离目录也从这里派生（见 session.rs 的 profile_dir）。
pub fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let dir = if cfg!(debug_assertions) { base.join("dev") } else { base };
    // 演示模式整个换一个根：假账号库、cookie 快照、会话隔离目录全落在这里，碰不到真实数据
    let dir = if demo::on() { dir.join("demo") } else { dir };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// 下载落在哪。演示模式放进演示目录：点"在访达中显示"时，真实「下载」目录里的文件不能露给观众
pub fn downloads_dir(app: &AppHandle) -> Result<PathBuf, String> {
    if demo::on() {
        let dir = data_dir(app)?.join("downloads");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        return Ok(dir);
    }
    if let Some(dir) = custom_download_dir(app) {
        return Ok(dir);
    }
    app.path().download_dir().map_err(|e| e.to_string())
}

/// 设置里选的下载文件夹（存在加密库的 meta 里，空串 = 用系统「下载」目录）
const DOWNLOAD_DIR_KEY: &str = "download_dir";
/// "1" = 每次下载完都弹保存框问存到哪
const DOWNLOAD_ASK_KEY: &str = "download_ask";

/// 用户选过、而且现在还在的下载文件夹。被删了、改名了、外接盘拔了就当没选，
/// 退回系统「下载」目录——不然下载会静默失败。锁着的时候读不到库，也按没选算
fn custom_download_dir(app: &AppHandle) -> Option<PathBuf> {
    let vault = app.try_state::<Vault>()?;
    let dir = PathBuf::from(with_conn(&vault, |c| db::get_meta(c, DOWNLOAD_DIR_KEY)).ok()?);
    dir.is_dir().then_some(dir)
}

/// 下载完要不要弹保存框。演示模式永远不问：保存框里会露出真实的文件夹
pub fn download_ask(app: &AppHandle) -> bool {
    if demo::on() {
        return false;
    }
    let Some(vault) = app.try_state::<Vault>() else { return false };
    with_conn(&vault, |c| db::get_meta(c, DOWNLOAD_ASK_KEY)).is_ok_and(|v| v == "1")
}

/// 这次运行里本 App 下载 / 另存过的文件。「在访达中显示」「另存到…」只认这些路径：
/// 路径是前端传回来的，不能让它拿去挪任意文件、露任意文件
static DOWNLOADED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

pub fn remember_download(p: &Path) {
    if let Ok(mut v) = DOWNLOADED.lock() {
        v.push(p.to_path_buf());
    }
}

fn is_our_download(p: &Path) -> bool {
    DOWNLOADED.lock().is_ok_and(|v| v.iter().any(|x| x == p))
}

fn db_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(data_dir(app)?.join(DB_FILE))
}

fn owner_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(data_dir(app)?.join(OWNER_FILE))
}

fn write_owner(app: &AppHandle, name: &str) {
    if let Ok(p) = owner_path(app) {
        let _ = std::fs::write(p, name);
    }
}

/// 取已解锁的连接跑一段闭包，锁定态直接报错。
fn with_conn<T>(
    vault: &State<Vault>,
    f: impl FnOnce(&Connection) -> Result<T, String>,
) -> Result<T, String> {
    let guard = vault.0.lock().map_err(|e| e.to_string())?;
    let conn = guard.as_ref().ok_or("账号库未解锁")?;
    f(conn)
}

fn now_string() -> String {
    let t = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        t.year(),
        u8::from(t.month()),
        t.day(),
        t.hour(),
        t.minute()
    )
}

// ---------------------------------------------------------------- 身份 / 账号库

#[tauri::command]
fn is_initialized(app: AppHandle) -> Result<bool, String> {
    Ok(db_path(&app)?.exists())
}

#[tauri::command]
fn register(app: AppHandle, vault: State<Vault>, username: String, password: String) -> Result<(), String> {
    // 必须在后端也挡一道：空密码传给 SQLCipher 等于 `PRAGMA key = ''`，
    // 那样建出来的库**根本没有加密**，光靠前端校验挡不住。
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(format!("主密码至少 {MIN_PASSWORD_LEN} 位"));
    }
    let path = db_path(&app)?;
    if path.exists() {
        return Err("本机已经有账号库了，请直接登录".into());
    }
    let conn = db::open(&path, &password)?;
    db::set_meta(&conn, "username", &username)?;
    write_owner(&app, &username);
    // 跟界面读到的自动锁定时长对齐（新库没设过 = 默认 30 分钟），不然"超时后点一下不续命"那道判断在注册后一直不生效
    let limit = auto_lock_secs_of(&conn);
    *vault.0.lock().map_err(|e| e.to_string())? = Some(conn);
    if let Ok(mut c) = app.state::<Activity>().0.lock() {
        c.reset(limit);
    }
    Ok(())
}

#[derive(Serialize)]
struct UnlockOutcome {
    blocked: Option<Verdict>,
    mode: String,
}

/// `force` 是区域检测的逃生口，跟打开账号页那个"仍要继续"一样（docs/NETWORK-GUARD.md：两种模式都要给）。
/// 没有它会形成死结：误判（比如公司专线出口在境外）时解不开库，而检测模式就存在库里，
/// 想改成"仅提示"或"关闭"得先解锁——最后只剩敲终端命令清库这一条路。
/// 放行的只是本地读库；之后打开每个账号页照样过检测，运行中的页面照样受哨兵管。
#[tauri::command]
async fn unlock(app: AppHandle, vault: State<'_, Vault>, password: String, force: bool) -> Result<UnlockOutcome, String> {
    let path = db_path(&app)?;
    // SQLite 的 open 会自动建文件：库不在时不挡一下，这里会**静默造出一个空库**，
    // 用户看到的是"登录成功但账号全没了"，比直接报错难查得多。
    if !path.exists() {
        return Err("本机还没有账号库，请先创建主账号".into());
    }
    let conn = db::open_for_unlock(&path, &password)?;
    let (mode, endpoints) = region_settings_from_conn(&conn);
    if mode != "off" && !force {
        // 不把连接交给 Vault，前端就无法读取或处理任何账号数据。
        // 解锁时强制重测，避免沿用锁库前的旧结论。
        let verdict = region_verdict(&app, &endpoints, true).await;
        if verdict.blocked() {
            return Ok(UnlockOutcome { blocked: Some(verdict), mode });
        }
    }
    db::prepare(&conn)?;
    // 老库（或刚导入的备份）可能还没有库外这份用户名，解锁成功时顺手补上，
    // 下次登录页就能显示出来
    if let Ok(name) = db::get_meta(&conn, "username") {
        write_owner(&app, &name);
    }
    let limit = auto_lock_secs_of(&conn);
    *vault.0.lock().map_err(|e| e.to_string())? = Some(conn);
    if let Ok(mut c) = app.state::<Activity>().0.lock() {
        c.reset(limit);
    }
    let watch = app.state::<RegionWatch>();
    watch.secs.store(region_watch_secs(&vault), Ordering::Relaxed);
    watch.changed.notify_one();
    Ok(UnlockOutcome { blocked: None, mode })
}

/// 登录页显示"当前账号是谁"。没有就返回空串（老库还没解锁过一次）。
#[tauri::command]
fn current_owner(app: AppHandle) -> String {
    owner_path(&app)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// 锁库的收尾：把 cookie 抄进加密库 → 抹掉 WebKit 留在磁盘上的登录数据 → 关页面 → 放开库。
///
/// 不抹的话，长期有效的 cookie、LocalStorage 以**明文**躺在 ~/Library/WebKit 底下，
/// 锁不锁库都一样能被捡走，加密库里那份快照等于白存。代价是把登录凭证放在 LocalStorage 里的站
/// 锁一次就得重新登（cookie 登录的站由快照接回来）。锁定、清空、导入备份都走这里。
/// ⌘Q 退出不经过这条路（见 run() 里的注释），磁盘上的登录数据会留到下次锁定。
///
/// `keep_alive` 为真（手动锁定、自动锁定）时，开了保活的会话**不关不抹**，藏在锁屏后面接着刷新：
/// 保活的意思就是 App 开着就一直在线，锁一下就掉等于没开。这是用户按账号勾的，代价是这几个账号的
/// 登录数据在锁定期间留在磁盘上。库照样锁上，密钥照样放掉。清空账号库、导入备份、演示模式传 false
fn lock_down(app: &AppHandle, vault: &State<Vault>, keep_alive: bool) -> Result<(), String> {
    snapshot_cookies(app, None);
    let kept = if keep_alive { session::keep_alive_ids(app) } else { Vec::new() };
    let ids: Vec<String> = with_conn(vault, db::list_accounts)
        .map(|v| v.into_iter().map(|a| a.id).filter(|id| !kept.contains(id)).collect())
        .unwrap_or_default();
    session::purge(app, &ids);
    session::close_all(app, &kept)?;
    *vault.0.lock().map_err(|e| e.to_string())? = None;
    // 「仍要继续」只管这一次解锁期间
    if let Ok(mut g) = app.state::<RegionGrace>().0.lock() {
        *g = None;
    }
    Ok(())
}

#[tauri::command]
fn lock(app: AppHandle, vault: State<Vault>) -> Result<(), String> {
    // 演示库的会话不留：锁定即退出演示
    lock_down(&app, &vault, !demo::on())?;
    // 锁定即退出演示，回到真实账号库的登录页
    demo::set(false);
    Ok(())
}

/// 写剪贴板。WKWebView 里 navigator.clipboard / execCommand("copy") 只认"用户点击的当下"，
/// 复制密码要先 await 验主密码、再 await 取明文，手势早过期了，写入十有八九被拒。
/// 前端写不进去就走这里（见 ui.tsx 的 copyText）
#[tauri::command]
async fn write_clipboard(text: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        #[cfg(target_os = "macos")]
        {
            use std::io::Write;
            use std::process::{Command, Stdio};
            // 跟下面一样：不给 LANG 的话 pbcopy 按 MacRoman 走，中文会乱
            let mut c = Command::new("pbcopy").env("LANG", "en_US.UTF-8").stdin(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
            c.stdin.take().ok_or("pbcopy 没有 stdin")?.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
            return c.wait().map_err(|e| e.to_string())?.success().then_some(()).ok_or_else(|| "pbcopy 失败".into());
        }
        // 值走环境变量，理由同 clear_clipboard_later
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            let ok = std::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "Add-Type -AssemblyName System.Windows.Forms; [Windows.Forms.Clipboard]::SetText($env:SB_CLIP)",
                ])
                .env("SB_CLIP", &text)
                .creation_flags(CREATE_NO_WINDOW)
                .status()
                .map_err(|e| e.to_string())?
                .success();
            return ok.then_some(()).ok_or_else(|| "写剪贴板失败".into());
        }
        #[allow(unreachable_code)]
        {
            let _ = text;
            Err("这个系统上不支持".into())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 明文进剪贴板 30 秒后清掉——前提是剪贴板里还是这一段，用户这期间又复制了别的就不动。
/// 前端 navigator.clipboard 读剪贴板会在 WKWebView 里弹"粘贴"确认，所以在这边用 pbpaste 比对
#[tauri::command]
fn clear_clipboard_later(text: String) {
    #[cfg(target_os = "macos")]
    std::thread::spawn(move || {
        use std::process::{Command, Stdio};
        std::thread::sleep(std::time::Duration::from_secs(30));
        // GUI 进程没有 LANG，不指定的话 pbpaste/pbcopy 按 MacRoman 走，中文密码就比不上
        let current = Command::new("pbpaste").env("LANG", "en_US.UTF-8").output().map(|o| o.stdout).unwrap_or_default();
        if current == text.as_bytes() {
            // pbcopy 喂一段空输入 = 清空
            if let Ok(mut c) = Command::new("pbcopy").env("LANG", "en_US.UTF-8").stdin(Stdio::piped()).spawn() {
                drop(c.stdin.take());
                let _ = c.wait();
            }
        }
    });
    // Windows：值走环境变量，不拼进命令行——不用操心转义，也不会出现在进程列表的命令行里。
    // 剪贴板 API 要求 STA 线程，powershell.exe 默认就是。Chromium 写剪贴板时会把 \n 换成 \r\n，比对前换回来
    #[cfg(target_os = "windows")]
    std::thread::spawn(move || {
        use std::os::windows::process::CommandExt;
        std::thread::sleep(std::time::Duration::from_secs(30));
        let _ = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Add-Type -AssemblyName System.Windows.Forms; \
                 $c = [Windows.Forms.Clipboard]::GetText() -replace \"`r`n\", \"`n\"; \
                 if ($c -ceq $env:SB_CLIP) { [Windows.Forms.Clipboard]::Clear() }",
            ])
            .env("SB_CLIP", &text)
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    });
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = text;
}

/// 进演示模式：换到独立的 demo/ 数据目录，清空重建一个假账号库直接解锁，登录页全是本机模拟的。
/// 只能从登录页进——真实库开着的时候切过去，会话和 cookie 快照会串库。
#[tauri::command]
fn enter_demo(app: AppHandle, vault: State<Vault>) -> Result<(), String> {
    let mut guard = vault.0.lock().map_err(|e| e.to_string())?;
    if guard.is_some() {
        return Err("先锁定当前账号库，再进演示模式".into());
    }
    // 锁定时留下的保活会话属于真实账号库，不能带进演示（演示是给别人看的）。趁数据目录还没切走先抹掉
    let kept = session::keep_alive_ids(&app);
    session::purge(&app, &kept);
    let port = demo::serve()?;
    demo::set(true);
    let built = (|| {
        let dir = data_dir(&app)?;
        // 要整个删的目录必须是演示目录，哪天 data_dir 改坏了也不能删到真实账号库
        if !dir.ends_with("demo") {
            return Err(format!("演示目录不对：{}", dir.display()));
        }
        // 上一轮演示留下的库、cookie 快照、下载全清掉，每次都从同一份假数据开始。
        // Windows 上刚退出的演示页面，WebView2 还会占着 profiles/ 几秒，整个目录可能删不掉——
        // 删不干净不要紧（上一轮的模拟登录态靠换 token 作废，见 demo.rs），旧的演示账号库必须删掉
        let _ = std::fs::remove_dir_all(&dir);
        let db = db_path(&app)?;
        if db.exists() {
            std::fs::remove_file(&db).map_err(|e| format!("清不掉上一轮的演示账号库：{e}"))?;
        }
        let conn = db::open(&db, demo::PASSWORD)?;
        db::set_meta(&conn, "username", demo::OWNER)?;
        // 演示承诺不联网：区域检测关掉（关 = 一个探测请求都不发）。要演示这项功能就去设置里开，只改演示库
        db::set_meta(&conn, MODE_KEY, "off")?;
        // 演示时常常停下来讲，别讲着讲着被自动锁定（锁定 = 退出演示）
        db::set_meta(&conn, AUTO_LOCK_SECS_KEY, "0")?;
        for a in demo::accounts(port) {
            db::save_account(&conn, &a)?;
        }
        write_owner(&app, demo::OWNER);
        Ok(conn)
    })();
    match built {
        Ok(conn) => {
            *guard = Some(conn);
            if let Ok(mut c) = app.state::<Activity>().0.lock() {
                c.reset(0);
            }
            Ok(())
        }
        Err(e) => {
            demo::set(false);
            Err(e)
        }
    }
}

/// 标一次"刚刚有人在动"。账号页面的 agent_js 和主窗口自己都会调；
/// 不读不写任何账号数据，所以账号页面（外部登录页）里也能安全地开放这条命令。
///
/// **已经超时的不续命**，返回 true 让调用方去锁。否则人离开半小时、回来随手一点，
/// 赶在前端下一轮检查之前把时钟刷新了，自动锁定就被一次点击绕过去了。
///
/// 是从哪个账号页面报上来的，就顺手给那个账号的保活重新计时（主窗口报的对不上任何页签，什么也不做）。
/// 账号认的是调用方自己的 webview，页面没法替别的账号报
#[tauri::command]
fn touch_activity(app: AppHandle, webview: Webview, activity: State<Activity>) -> bool {
    session::mark_input(&app, webview.label());
    activity.0.lock().map(|mut c| c.touch()).unwrap_or(false)
}

/// 距离上一次有效操作过了多少秒，前端拿它跟设置的时长比。
/// 还没记过（解锁时就会记，理论上碰不到）按"刚动过"算，不能读到空值就把人锁出去。
#[tauri::command]
fn idle_secs(activity: State<Activity>) -> u64 {
    activity.0.lock().map(|c| c.idle_secs()).unwrap_or(0)
}

fn auto_lock_secs_of(conn: &Connection) -> u64 {
    db::get_meta(conn, AUTO_LOCK_SECS_KEY)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&s| s == 0 || (60..=86_400).contains(&s))
        .unwrap_or(DEFAULT_AUTO_LOCK_SECS)
}

/// 没设过就是 30 分钟。解锁期间密钥和明文都在内存里，人走开了库还开着，加密等于没用；
/// 用户明确选过「关闭」（库里存的是 0）的照旧关着。
const DEFAULT_AUTO_LOCK_SECS: u64 = 30 * 60;

/// 多久无操作自动锁定，0 = 关闭。
#[tauri::command]
fn get_auto_lock_secs(vault: State<Vault>) -> Result<u64, String> {
    with_conn(&vault, |c| Ok(auto_lock_secs_of(c)))
}

#[tauri::command]
fn set_auto_lock_secs(vault: State<Vault>, activity: State<Activity>, secs: u64) -> Result<(), String> {
    if secs != 0 && !(60..=86_400).contains(&secs) {
        return Err("自动锁定时长须是 0（关闭）或 1 分钟到 24 小时之间".into());
    }
    with_conn(&vault, |c| db::set_meta(c, AUTO_LOCK_SECS_KEY, &secs.to_string()))?;
    // 正在点设置的人显然在场，顺手重新起算
    if let Ok(mut c) = activity.0.lock() {
        c.reset(secs);
    }
    Ok(())
}

/// 把明文放进剪贴板之前核一次主密码。
///
/// 不碰 Vault 那个已解锁的连接——单独拿候选密码开一次库，SQLCipher 的钥匙对不上
/// 就读不出 sqlite_master，报错即"密码不对"。顺带也就没法用这个命令读到任何账号数据。
#[tauri::command]
fn verify_master_password(app: AppHandle, password: String) -> Result<(), String> {
    let path = db_path(&app)?;
    if !path.exists() {
        return Err("本机还没有账号库".into());
    }
    db::open_for_unlock(&path, &password).map(|_| ())
}

#[tauri::command]
fn change_master_password(
    app: AppHandle,
    vault: State<Vault>,
    current: String,
    new_password: String,
) -> Result<(), String> {
    if new_password.chars().count() < MIN_PASSWORD_LEN {
        return Err(format!("新主密码至少 {MIN_PASSWORD_LEN} 位"));
    }
    // 解锁后内存里没有原密码，所以另开一条连接验一次，免得手滑把库改成谁也打不开
    drop(db::open(&db_path(&app)?, &current).map_err(|_| "当前主密码不正确".to_string())?);
    with_conn(&vault, |c| db::rekey(c, &new_password))
}

/// 忘记主密码时的自救提示。刻意做成"必须在数据目录里敲命令"，
/// 不给 UI 按钮——重置等于丢掉整个账号库，不该是一次误点就能触发的事。
#[derive(Serialize)]
struct ResetHint {
    data_dir: String,
    command: String,
}

/// 设置页"清空所有账户数据"走的就是这条路——跟 `reset` 子命令改名旧库的逻辑一样，
/// 只是从 UI 里点触发，不能再靠"要敲终端命令"这道天然的坎，所以门槛不能比
/// `change_master_password` 低：这是全 App 最不可逆的操作，必须重新验一次主密码，
/// 不能只让前端弹个"输入文字确认"就放行。
#[tauri::command]
fn wipe_vault(app: AppHandle, vault: State<Vault>, password: String) -> Result<(), String> {
    let path = db_path(&app)?;
    if !path.exists() {
        return Err("本机还没有账号库".into());
    }
    drop(db::open_for_unlock(&path, &password).map_err(|_| "主密码不正确".to_string())?);
    // 账号库作废，各账号在磁盘上的登录数据也一起抹掉，不然"清空"之后控制台还登着
    lock_down(&app, &vault, false)?;
    let _ = std::fs::remove_dir_all(data_dir(&app)?.join("profiles"));
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup = path.with_file_name(format!("{DB_FILE}.locked-{stamp}"));
    std::fs::rename(&path, &backup).map_err(|e| e.to_string())?;
    prune_archives(&data_dir(&app)?, "locked");
    let _ = std::fs::remove_file(owner_path(&app)?);
    Ok(())
}

#[tauri::command]
fn reset_hint(app: AppHandle) -> Result<ResetHint, String> {
    let dir = data_dir(&app)?;
    let command = if cfg!(windows) {
        // Windows 不走 `switchboard.exe reset`：正式版是 GUI 程序，在终端里跑它一句输出都没有、立刻返回，
        // 用户不知道成没成；Windows 默认终端又是 PowerShell，不认 `&&`。直接给一条 PowerShell 命令做同样的事：
        // 库改名归档（不删）、删掉库外那份用户名
        let d = ps_quote(&dir.to_string_lossy());
        format!(
            "Rename-Item -LiteralPath (Join-Path {d} '{DB_FILE}') -NewName ('{DB_FILE}.locked-' + [DateTimeOffset]::Now.ToUnixTimeSeconds()); \
             Remove-Item -LiteralPath (Join-Path {d} '{OWNER_FILE}') -ErrorAction SilentlyContinue; \
             '已重置：旧账号库已改名归档，重新打开 Switchboard 会回到创建主账号'"
        )
    } else {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        format!("cd {} && {} reset", sh_quote(&dir.to_string_lossy()), sh_quote(&exe.to_string_lossy()))
    };
    Ok(ResetHint { command, data_dir: dir.display().to_string() })
}

/// PowerShell 的单引号字符串：里面的 `'` 写成 `''`，其余一律按字面，`$`、反引号都不展开
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// POSIX shell 的单引号转义。这条命令是给用户粘进终端的：用 `{:?}` 的双引号的话，
/// App 装在名字带 `$` 或反引号的文件夹里时，shell 会先展开/执行它们
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

// ---------------------------------------------------------------- 网络区域检测

/// 读设置。注意**同步读完立刻放锁**——后面要 await，不能把 std 的 MutexGuard 带过去。
fn region_settings_from_conn(conn: &Connection) -> (String, Vec<Endpoint>) {
    let mode = db::get_meta(conn, MODE_KEY).unwrap_or_else(|_| "block".into());
    let endpoints = db::get_meta(conn, ENDPOINTS_KEY)
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<Endpoint>>(&s).ok())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(region::default_endpoints);
    (mode, endpoints)
}

/// 最近一次从库里读到的区域检测设置。锁定时保活的会话还开着（见 `lock_down`），哨兵得照用户的设置管它们，
/// 库锁着读不到，就用这份；从来没读到过才按最保守的 block 算
static LAST_REGION_SETTINGS: Mutex<Option<(String, Vec<Endpoint>)>> = Mutex::new(None);

fn region_settings(vault: &State<Vault>) -> (String, Vec<Endpoint>) {
    let fallback = || {
        LAST_REGION_SETTINGS.lock().ok().and_then(|g| g.clone())
            .unwrap_or_else(|| ("block".into(), region::default_endpoints()))
    };
    let Ok(guard) = vault.0.lock() else { return fallback() };
    let Some(conn) = guard.as_ref() else { return fallback() };
    let settings = region_settings_from_conn(conn);
    if let Ok(mut g) = LAST_REGION_SETTINGS.lock() {
        *g = Some(settings.clone());
    }
    settings
}

fn region_watch_secs(vault: &State<Vault>) -> u64 {
    with_conn(vault, |conn| Ok(db::get_meta(conn, WATCH_SECS_KEY).ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|s| (60..=86_400).contains(s))
        .unwrap_or(DEFAULT_WATCH_SECS)))
        .unwrap_or(DEFAULT_WATCH_SECS)
}

/// 取检测结论。`force` 为真时忽略缓存重新探测。
async fn region_verdict(app: &AppHandle, endpoints: &[Endpoint], force: bool) -> Verdict {
    let cache = app.state::<RegionCache>();
    if !force {
        if let Ok(g) = cache.0.lock() {
            if let Some((v, at)) = g.as_ref() {
                if at.elapsed().as_secs() < REGION_CACHE_SECS {
                    return v.clone();
                }
            }
        }
    }
    let v = region::check(endpoints).await;
    if let Ok(mut g) = cache.0.lock() {
        *g = Some((v.clone(), std::time::Instant::now()));
    }
    v
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RegionState {
    mode: String,
    verdict: Verdict,
    endpoints: Vec<Endpoint>,
    /// 当前是不是已经被切断（页面全被导航到 about:blank）
    cut: bool,
    watch_secs: u64,
}

// ---------------------------------------------------------------- 定时哨兵

/// 哨兵的记忆：用来分辨"状态变了"和"一直这样"。
/// 没有它，warn 模式会每轮弹一次框，block 模式会反复切断已经断开的页面。
#[derive(Default)]
struct RegionGuard(Mutex<bool>); // 上一次探测是否需要区域保护处理

/// 推给前端的实时通知
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RegionEvent {
    verdict: Verdict,
    mode: String,
    /// 要不要弹框。判断在后端做（见 `decide`），前端照做就行——
    /// 两边各判一次迟早会不一致
    ask: bool,
    cut: bool,
}

/// 哨兵探完一轮之后该干什么。抽成纯函数是因为这张表有四个输入，
/// 错一格的后果是"该断的没断"或者"每轮弹一次框"，肉眼看不出来。
#[derive(Debug, PartialEq)]
enum Act {
    /// 立刻切断所有页面
    Cut,
    /// 接回被切断的页面
    Restore,
    /// 弹框问用户
    Ask,
    Nothing,
}

/// - `blocked`：这一轮探到境外出口，或大陆出口与可达的境外探针并存
/// - `cut`：当前页面是不是已经处于切断状态
/// - `changed`：跟上一轮相比状态翻转了
/// - `grace`：用户刚在拦截框里点过「仍要继续」，还在宽限期里（见 `FORCE_GRACE_SECS`）
fn decide(mode: &str, blocked: bool, cut: bool, changed: bool, grace: bool) -> Act {
    // 网络恢复正常：不管什么模式都得把断掉的接回来，否则用户永远卡在空白页
    if !blocked {
        return if cut { Act::Restore } else { Act::Nothing };
    }
    // 用户刚明确说了"知道，照样开"，宽限期里不断也不问；过了宽限期按模式正常处理
    if grace {
        return Act::Nothing;
    }
    match mode {
        // 拦截模式：探到异常就断，不商量。已经断了就别每轮重断一次
        "block" => {
            if cut {
                Act::Nothing
            } else {
                Act::Cut
            }
        }
        // 提示模式：只在状态**翻转**的那一次问。异常持续时不要反复骚扰。
        // 已经断了就没什么可问的——问"要不要断开"只会让人莫名其妙
        "warn" => {
            if changed && !cut {
                Act::Ask
            } else {
                Act::Nothing
            }
        }
        // off 在调用处就返回了，走不到这里
        _ => Act::Nothing,
    }
}

/// 一次哨兵巡检。定时器和"用户手动重新检测"都走这里，保证处置逻辑只有一份。
async fn region_patrol(app: &AppHandle) {
    let (mode, endpoints) = region_settings(&app.state::<Vault>());
    // 关闭 = 一个探测请求都不发。这是设置项里写死的承诺，不能偷偷探
    if mode == "off" {
        return;
    }
    // 锁着又没有页面开着，探了也没东西可处置，白烧请求。
    // 锁着但有页面开着 = 锁定时留下的保活会话，照样得管：网络变了该断就断
    let locked = app.state::<Vault>().0.lock().map(|g| g.is_none()).unwrap_or(true);
    if locked && session::keep_alive_ids(app).is_empty() {
        return;
    }

    let verdict = region::check(&endpoints).await;
    // 覆盖缓存：打开账号时读的是同一份，别让哨兵和开门校验各说各话
    if let Ok(mut g) = app.state::<RegionCache>().0.lock() {
        *g = Some((verdict.clone(), std::time::Instant::now()));
    }

    let blocked = verdict.blocked();
    let changed = {
        let guard = app.state::<RegionGuard>();
        let mut last = match guard.0.lock() {
            Ok(l) => l,
            Err(_) => return,
        };
        let changed = *last != blocked;
        // 锁屏上问不了人：翻转留到解锁后那一轮再算，warn 模式才会问到
        if !locked {
            *last = blocked;
        }
        changed
    };

    let mut ask = false;
    let act = decide(&mode, blocked, session::any_cut(app), changed, grace_active(app));
    match act {
        // 先抄 cookie 再断——导航走会丢掉内存里的登录态
        Act::Cut => {
            snapshot_cookies(app, None);
            let n = session::cut_all(app);
            eprintln!("[switchboard] 区域拦截：已切断 {n} 个会话（出口 {}）", verdict.ip);
            // 断完也要弹框：页面无缘无故变空白，不说一声用户只会以为程序坏了。
            // 只有这一次会弹——下一轮 cut 已经是 true，判定成 Nothing
            ask = true;
        }
        // 自动接回去。要求用户手动点一次更"稳"，但网络抖一下就得人来救一次，实际很烦
        Act::Restore => {
            let n = session::restore_all(app);
            eprintln!("[switchboard] 区域恢复：已接回 {n} 个会话");
        }
        Act::Ask => ask = true,
        Act::Nothing => {}
    }

    let _ = app.emit_to(
        tauri::EventTarget::labeled("main"),
        "region-changed",
        RegionEvent { verdict, mode, ask: ask && !locked, cut: session::any_cut(app) },
    );
}

/// warn 模式下用户选了"断开"，或用户想手动断。
#[tauri::command]
fn cut_sessions(app: AppHandle) -> usize {
    snapshot_cookies(&app, None);
    session::cut_all(&app)
}

/// 用户说"我已经关了代理"：强制重新探测，确认不再触发区域保护才接回被切断的会话。
///
/// 刻意不提供裸的 restore 命令——那等于给前端一个绕过拦截的开关，
/// 点一下就能在境外网络下把页面全接回来，这道闸就白设了。
#[tauri::command]
async fn recheck_and_restore(app: AppHandle, vault: State<'_, Vault>) -> Result<RegionState, String> {
    let (mode, endpoints) = region_settings(&vault);
    let verdict = region_verdict(&app, &endpoints, true).await;
    if !verdict.blocked() {
        session::restore_all(&app);
    }
    if let Ok(mut g) = app.state::<RegionGuard>().0.lock() {
        *g = verdict.blocked();
    }
    let cut = session::any_cut(&app);
    Ok(RegionState { mode, verdict, endpoints, cut, watch_secs: region_watch_secs(&vault) })
}

#[tauri::command]
async fn check_region(app: AppHandle, vault: State<'_, Vault>, force: bool) -> Result<RegionState, String> {
    let (mode, endpoints) = region_settings(&vault);
    // 关闭模式下页面初始化和设置页加载不能偷偷发送探测请求。
    let verdict = if mode == "off" && !force {
        Verdict { origin: Origin::Unknown { reason: "已关闭检测".into() }, tunnel: false, proxy: false, ip: String::new() }
    } else {
        region_verdict(&app, &endpoints, force).await
    };
    let cut = session::any_cut(&app);
    Ok(RegionState { mode, verdict, endpoints, cut, watch_secs: region_watch_secs(&vault) })
}

/// 用户主动检测时走同一条哨兵处置路径，异常网络立即按当前模式处理。
#[tauri::command]
async fn patrol_region(app: AppHandle, vault: State<'_, Vault>) -> Result<RegionState, String> {
    let (mode, endpoints) = region_settings(&vault);
    let unlocked = vault.0.lock().map_err(|e| e.to_string())?.is_some();
    if mode != "off" && unlocked {
        region_patrol(&app).await;
    }
    // 关闭自动检测时，用户显式点击仍可做一次手动探测。
    let verdict = region_verdict(&app, &endpoints, mode == "off" || !unlocked).await;
    Ok(RegionState {
        mode,
        verdict,
        endpoints,
        cut: session::any_cut(&app),
        watch_secs: region_watch_secs(&vault),
    })
}

#[tauri::command]
fn set_region_watch_secs(app: AppHandle, vault: State<Vault>, secs: u64) -> Result<(), String> {
    if !(60..=86_400).contains(&secs) {
        return Err("复查间隔须在 1 分钟到 24 小时之间".into());
    }
    with_conn(&vault, |c| db::set_meta(c, WATCH_SECS_KEY, &secs.to_string()))?;
    let watch = app.state::<RegionWatch>();
    watch.secs.store(secs, Ordering::Relaxed);
    watch.changed.notify_one();
    Ok(())
}

#[tauri::command]
fn set_region_mode(app: AppHandle, vault: State<Vault>, mode: String) -> Result<(), String> {
    if !matches!(mode.as_str(), "block" | "warn" | "off") {
        return Err(format!("不认识的模式：{mode}"));
    }
    with_conn(&vault, |c| db::set_meta(c, MODE_KEY, &mode))?;
    if let Ok(mut cache) = app.state::<RegionCache>().0.lock() {
        *cache = None;
    }
    Ok(())
}

/// 保存探测端点。格式不合法就拒绝保存、保留原值——别让人把自己配到打不开 App。
#[tauri::command]
fn set_region_endpoints(vault: State<Vault>, json: String) -> Result<usize, String> {
    let parsed: Vec<Endpoint> =
        serde_json::from_str(json.trim()).map_err(|e| format!("JSON 格式不对：{e}"))?;
    if parsed.is_empty() {
        return Err("至少要留一个端点".into());
    }
    for e in &parsed {
        if !e.url.starts_with("https://") {
            return Err(format!("「{}」必须用 HTTPS——明文响应可被中间人伪造，这道闸就白设了", e.name));
        }
        if e.country_path.trim().is_empty() {
            return Err(format!("「{}」缺少 country_path", e.name));
        }
    }
    let n = parsed.len();
    let text = serde_json::to_string(&parsed).map_err(|e| e.to_string())?;
    with_conn(&vault, |c| db::set_meta(c, ENDPOINTS_KEY, &text))?;
    Ok(n)
}

#[tauri::command]
fn reset_region_endpoints(vault: State<Vault>) -> Result<String, String> {
    let text = serde_json::to_string_pretty(&region::default_endpoints()).map_err(|e| e.to_string())?;
    with_conn(&vault, |c| db::set_meta(c, ENDPOINTS_KEY, &text))?;
    Ok(text)
}

// ---------------------------------------------------------------- 账号 CRUD

#[tauri::command]
fn list_accounts(vault: State<Vault>) -> Result<Vec<Account>, String> {
    with_conn(&vault, db::list_accounts).map(|v| v.into_iter().map(masked).collect())
}

/// 完整的一条，带明文。只在用户真要用的时候取：编辑、复制新建、复制到剪贴板
#[tauri::command]
fn get_account(vault: State<Vault>, id: String) -> Result<Account, String> {
    with_conn(&vault, |c| db::get_account(c, &id))
}

/// 列表里的密文字段一律换成它，只告诉界面"有没有"
const MASK: &str = "••••••";

/// 账号列表给界面用：密码、TOTP 密钥、标了敏感的附加字段打码。
/// 以前解锁后全部明文一次性下发，整个解锁期间都躺在界面的内存里；
/// 现在要明文时单独取（get_account / fill_focused），用完就没了
fn masked(mut a: Account) -> Account {
    let hide = |v: &mut String| {
        if !v.is_empty() {
            *v = MASK.into();
        }
    };
    hide(&mut a.credential);
    hide(&mut a.totp_secret);
    a.extra_fields.iter_mut().filter(|f| f.secret).for_each(|f| hide(&mut f.value));
    a
}

/// 存之前把打码的值换回库里的原值。编辑页正常拿的是完整数据，碰不到这里；
/// 这是兜底：哪条路径漏了、把打过码的账号拿去存，也不能把真密码覆盖成一串圆点
fn unmask(conn: &Connection, a: &mut Account) -> Result<(), String> {
    let has_mask = a.credential == MASK || a.totp_secret == MASK || a.extra_fields.iter().any(|f| f.value == MASK);
    if !has_mask {
        return Ok(());
    }
    let redo = "账号数据不完整（带着打码的值），请重新打开编辑页再保存";
    let old = db::get_account(conn, &a.id).map_err(|_| redo.to_string())?;
    if a.credential == MASK {
        a.credential = old.credential;
    }
    if a.totp_secret == MASK {
        a.totp_secret = old.totp_secret;
    }
    for f in a.extra_fields.iter_mut().filter(|f| f.value == MASK) {
        f.value = old.extra_fields.iter().find(|o| o.label == f.label).map(|o| o.value.clone()).ok_or(redo)?;
    }
    Ok(())
}

#[tauri::command]
fn save_account(vault: State<Vault>, mut account: Account) -> Result<(), String> {
    with_conn(&vault, |c| {
        unmask(c, &mut account)?;
        db::save_account(c, &account)
    })
}

#[tauri::command]
fn delete_account(app: AppHandle, vault: State<Vault>, id: String) -> Result<(), String> {
    // 连同它在磁盘上的登录数据一起删，不然账号没了、控制台还登着
    session::purge(&app, std::slice::from_ref(&id));
    with_conn(&vault, |c| db::delete_account(c, &id))
}

/// 「其他」是兜底平台，底下常挂着好几个毫不相干的自建站（宝塔、Graylog……）。
/// 选择器按平台名共用一套的话，在一个站上指认会把别的站的覆盖掉，
/// 所以「其他」按站点单独存成 `其他 · host[:port]`，取的时候先找站点那条，没有再退回「其他」。
const OTHER_PLATFORM: &str = "其他";

fn config_key(a: &Account) -> String {
    if a.platform != OTHER_PLATFORM {
        return a.platform.clone();
    }
    let Ok(u) = a.login_url.trim().parse::<tauri::Url>() else { return a.platform.clone() };
    match (u.host_str(), u.port()) {
        (Some(h), Some(p)) => format!("{OTHER_PLATFORM} · {h}:{p}"),
        (Some(h), None) => format!("{OTHER_PLATFORM} · {h}"),
        _ => a.platform.clone(),
    }
}

fn config_for(conn: &Connection, a: &Account) -> Result<Option<PlatformConfig>, String> {
    let key = config_key(a);
    match db::get_platform_config(conn, &key)? {
        None if key != a.platform => db::get_platform_config(conn, &a.platform),
        found => Ok(found),
    }
}

/// 这个账号现在的自动填充脚本，连同它的登录地址（只在同站页面上填，见 session.rs 的 fill_allowed）。
///
/// 页面每次加载完都现取，不在开页面时拼死：页面开着的时候账号密码、平台选择器都可能被改
/// （编辑账号、指认输入框、改平台配置），拼死的话改完得关掉重开才生效；
/// 也省得明文密码在页面整个生命周期里一直攥在回调的内存里。锁着的时候取不到，也就不填
pub fn autofill_for(app: &AppHandle, account_id: &str) -> Option<(tauri::Url, String)> {
    let vault = app.try_state::<Vault>()?;
    let (a, cfg) = with_conn(&vault, |c| {
        let a = db::get_account(c, account_id)?;
        let cfg = config_for(c, &a)?;
        Ok((a, cfg))
    })
    .ok()?;
    let cfg = cfg.filter(|c| c.trigger_event != "skip" && !c.username_selector.is_empty() && !c.password_selector.is_empty())?;
    let login = a.login_url.trim().parse().ok()?;
    Some((login, adapter::fill_js(&cfg, &a.username, &a.credential)))
}

// ---------------------------------------------------------------- 会话

/// 打开账号的结果。`opened=false` 且 `blocked` 有值时，前端弹区域提示框，
/// 用户确认后带 `force=true` 再调一次。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenOutcome {
    opened: bool,
    blocked: Option<Verdict>,
    mode: String,
}

#[tauri::command]
async fn switch_or_open_account(
    app: AppHandle,
    vault: State<'_, Vault>,
    account_id: String,
    force: bool,
) -> Result<OpenOutcome, String> {
    // 这是"打开/切换账号页面"的唯一入口——主界面一键登录、会话页左栏、⌘K
    // 全都汇到这里，所以区域检查放这一处就够，放别处等于漏。
    let (mode, endpoints) = region_settings(&vault);
    // 只在**新建**页面时检查，已经开着的页面直接放行切换。
    // 运行中网络变了不归这里管——那是哨兵（region_patrol）按配置间隔巡检的事，
    // 探到异常会按模式实时切断。所以这里只管"开门"那一下。
    let already_open = session::is_open(&app, &account_id);
    // ponytail: 宽限只看时间，宽限期里网络从"分流代理"变成彻底的境外也不会断；真要分得这么细再按 verdict 记
    if force && mode != "off" {
        if let Ok(mut g) = app.state::<RegionGrace>().0.lock() {
            *g = Some(std::time::Instant::now());
        }
    }
    if mode != "off" && !force && !already_open {
        let v = region_verdict(&app, &endpoints, false).await;
        if v.blocked() {
            return Ok(OpenOutcome { opened: false, blocked: Some(v), mode });
        }
    }

    let (account, cookies, keep_alive) = with_conn(&vault, |c| {
        let a = db::get_account(c, &account_id)?;
        let cookies: Vec<String> = db::load_cookies(c, &account_id)?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Ok((a, cookies, keep_alive_map(c).get(&account_id).copied()))
    })?;
    session::open_or_focus(&app, &account, &cookies, keep_alive)?;
    Ok(OpenOutcome { opened: true, blocked: None, mode })
}

/// 把当前所有会话的 cookie 抄进加密库。退出 App、锁库、关单个会话时都要做一次，
/// 否则会话 cookie 随 WebView 一起消失，下次开又得重新登录。
fn snapshot_cookies(app: &AppHandle, only: Option<&str>) {
    let Some(vault) = app.try_state::<Vault>() else { return };
    let mut saved = 0usize;
    let ids: Vec<String> = match only {
        Some(id) => vec![id.to_string()],
        None => session::list(app).unwrap_or_default().into_iter().map(|s| s.account_id).collect(),
    };
    let now = now_string();
    for id in ids {
        let cookies = session::read_cookies(app, &id);
        if cookies.is_empty() {
            continue;
        }
        let Ok(json) = serde_json::to_string(&cookies) else { continue };
        if with_conn(&vault, |c| db::save_cookies(c, &id, &json, &now)).is_ok() {
            saved += 1;
        }
    }
    if saved > 0 {
        // 这条日志是"到底有没有抄成"的唯一凭据，别删
        eprintln!("[switchboard] cookie 快照：{saved} 个账号已存入加密库");
    }
}

fn keep_alive_map(conn: &Connection) -> std::collections::HashMap<String, bool> {
    db::get_meta(conn, KEEP_ALIVE_KEY).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// 会话页左栏的「保活」开关。`any_network` = 网络异常时也照样保活（默认关：异常就暂停）
#[tauri::command]
fn set_keep_alive(app: AppHandle, vault: State<Vault>, account_id: String, on: bool, any_network: bool) -> Result<(), String> {
    with_conn(&vault, |c| {
        let mut m = keep_alive_map(c);
        if on {
            m.insert(account_id.clone(), any_network);
        } else {
            m.remove(&account_id);
        }
        db::set_meta(c, KEEP_ALIVE_KEY, &serde_json::to_string(&m).map_err(|e| e.to_string())?)
    })?;
    session::set_keep_alive(&app, &account_id, on.then_some(any_network));
    Ok(())
}

/// 保活的一轮，挂在 cookie 快照线程上跑。锁着也跑：锁定时保活的会话留着（见 `lock_down`）。
///
/// 网络异常 = 区域检测开着、且最近一次结论要拦。结论最多是一个哨兵周期前的——
/// 跟区域拦截本身一样，网络在两次巡检之间变了是看不见的
fn keep_alive_tick(app: &AppHandle) {
    let vault = app.state::<Vault>();
    let abnormal = region_settings(&vault).0 != "off"
        && app.state::<RegionCache>().0.lock().is_ok_and(|g| g.as_ref().is_some_and(|(v, _)| v.blocked()));
    let n = session::keep_alive(app, abnormal);
    if n > 0 {
        eprintln!("[switchboard] 保活：刷新了 {n} 个页签");
    }
}

#[tauri::command]
fn list_active_sessions(app: AppHandle) -> Result<Vec<SessionInfo>, String> {
    session::list(&app)
}

#[tauri::command]
fn close_session(app: AppHandle, account_id: String) -> Result<(), String> {
    snapshot_cookies(&app, Some(&account_id));
    session::close(&app, &account_id)
}

/// 清掉这个账号保存的登录状态：开着的页面关掉，WebKit 的登录数据和加密库里的 cookie 快照一起删，
/// 下次打开就是没登录的样子。
///
/// 为什么需要：在网站上退出登录后 cookie 可能一条都不剩，而快照遇到空的会跳过（免得读取失败时误删有效登录），
/// 旧快照就一直留着，下次打开又被塞回去，服务端会话还没失效的话就又"自动登录"了
#[tauri::command]
fn clear_login(app: AppHandle, vault: State<Vault>, account_id: String) -> Result<(), String> {
    // 先关页面再删快照：顺序反了，中间定时快照可能又把 cookie 抄回来
    session::purge(&app, std::slice::from_ref(&account_id));
    with_conn(&vault, |c| db::delete_cookies(c, &account_id))
}

/// 重新加载账号页面。加载失败（限 IP、连不上）后的「刷新」走这里。
#[tauri::command]
fn reload_session(app: AppHandle, vault: State<Vault>, account_id: String) -> Result<(), String> {
    let account = with_conn(&vault, |c| db::get_account(c, &account_id))?;
    if account.login_url.trim().is_empty() {
        return Err("这个账号还没填登录直达 URL".into());
    }
    session::reload(&app, &account_id, &account.login_url)
}

/// 页内查找（会话页的「查找」按钮、主窗口里的 ⌘F）。焦点在页面里时按 ⌘F 不走这里，页面里的 agent 自己开
#[tauri::command]
fn find_in_page(app: AppHandle, account_id: String) -> Result<(), String> {
    session::find_in_page(&app, &account_id)
}

/// 当前页签的前进 / 后退。wry 没给 go_back / go_forward，`history.go` 就够用。
#[tauri::command]
fn tab_history(app: AppHandle, account_id: String, delta: i32) -> Result<(), String> {
    if delta != 1 && delta != -1 {
        return Err("一次只能前进或后退一步".into());
    }
    session::go_history(&app, &account_id, delta)
}

/// 在系统浏览器里打开一个地址（页签里打不开、要用插件/扫码的场合）。
///
/// **必须过区域检测**：系统浏览器完全不受我们控制，出口不合规时一键跳出去，
/// 等于给区域拦截开了个后门（见 docs/NETWORK-GUARD.md）。所以这里跟"打开账号页"同一道闸。
#[tauri::command]
async fn open_in_system_browser(
    app: AppHandle,
    vault: State<'_, Vault>,
    url: String,
) -> Result<(), String> {
    let parsed: tauri::Url = url.trim().parse().map_err(|_| format!("地址不合法：{url}"))?;
    // 只放 http(s)：file:// / 自定义 scheme 交给系统去开，等于把本机当命令行使
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("只支持 http / https 地址".into());
    }
    let (mode, endpoints) = region_settings(&vault);
    // 发行版页面不是云控制台，不拦（设置页「打开发行版页面」走这里）
    if mode != "off" && parsed.as_str() != RELEASE_PAGE {
        let v = region_verdict(&app, &endpoints, false).await;
        if v.blocked() {
            let where_ = match &v.origin {
                Origin::Outside { country, region } if !region.is_empty() && region != country => {
                    format!("{country} · {region}")
                }
                Origin::Outside { country, .. } => country.clone(),
                _ => "中国大陆 · 检测到分流代理 / VPN".to_string(),
            };
            return Err(format!(
                "当前网络出口是「{where_}」，不给在系统浏览器里打开——那边不受本 App 的区域拦截管。先处理网络再试"
            ));
        }
    }
    let arg = parsed.as_str();
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(arg);
        c
    };
    // 不能走 `cmd /C start`：地址是页面给的，cmd 会把里面的 `&` `|` 当命令分隔符，
    // `https://x.com/?a&calc` 就等于顺手执行了 calc。
    // 也不用 explorer：它会把地址里的逗号当成自己的参数分隔符（`/select,` 那套语法）。
    // rundll32 url.dll,FileProtocolHandler 是 Windows 上打开网址的通行做法，同样不过 shell 解析
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", arg]);
        c
    };
    #[cfg(target_os = "linux")]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(arg);
        c
    };
    cmd.spawn().map(|_| ()).map_err(|e| format!("打不开系统浏览器：{e}"))
}

/// 跟前端 DownloadGate.tsx 的 RELEASE_PAGE 是同一个地址
const RELEASE_PAGE: &str = "https://gitee.com/etn/switchboard_store/releases";

/// 一键自动演示开始 / 结束。开着的时候模拟登录页填好就自己提交（见 demo.rs）
#[tauri::command]
fn set_demo_tour(on: bool) -> Result<(), String> {
    if on && !demo::on() {
        return Err("只能在演示模式里自动演示".into());
    }
    demo::set_tour(on);
    Ok(())
}

/// 录主窗口这一块，`secs` 秒后自动结束。只录窗口不录整屏：别的窗口、通知不能进视频
#[tauri::command]
fn start_recording(app: AppHandle, secs: u64) -> Result<(), String> {
    if !demo::on() {
        return Err("只能在演示模式里录屏——真实账号库的界面不该进视频".into());
    }
    if !(5..=600).contains(&secs) {
        return Err("录屏时长须在 5 秒到 10 分钟之间".into());
    }
    let win = app.get_window("main").ok_or("找不到主窗口")?;
    let scale = win.scale_factor().map_err(|e| e.to_string())?;
    // ponytail: 按窗口所在屏的缩放换算；窗口横跨两块缩放不同的屏时会偏，真碰上再按屏分别算
    let p = win.outer_position().map_err(|e| e.to_string())?.to_logical::<f64>(scale);
    let s = win.outer_size().map_err(|e| e.to_string())?.to_logical::<f64>(scale);
    let rect = format!("{},{},{},{}", p.x.round(), p.y.round(), s.width.round(), s.height.round());
    let dir = app.path().video_dir().map_err(|e| e.to_string())?;
    let stamp = now_string().replace([' ', ':'], "-");
    demo::start_recording(&rect, dir.join(format!("Switchboard-演示-{stamp}.mov")), secs)
}

/// `keep=true` 等录完并在访达里选中视频，返回路径；`false` 丢掉这段
#[tauri::command]
async fn stop_recording(keep: bool) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || demo::stop_recording(keep))
        .await
        .map_err(|e| e.to_string())?
        .map(|p| p.display().to_string())
}

/// 在访达 / 资源管理器里选中页签刚下载的文件，否则用户只知道"下完了"，找不到它。
///
/// 路径是前端传回来的，只认这次运行里本 App 下载 / 另存过、而且还在的文件（见 `remember_download`）。
/// 不按"在不在下载目录里"判：用户可能中途改了下载文件夹，或者每次都另存到别处
#[tauri::command]
fn reveal_download(path: String) -> Result<(), String> {
    let path = PathBuf::from(&path);
    let file = Some(&path)
        .filter(|p| is_our_download(p))
        .and_then(|p| p.canonicalize().ok())
        .filter(|p| p.is_file())
        .ok_or("文件不在了，可能被移走或删掉")?;
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg("-R").arg(&file).spawn();
    // 资源管理器只认 /select,"路径" 这一种写法：整个参数被加上引号（路径有空格时 Rust 会这么做）它就不认了，
    // canonicalize 给的 \\?\ 前缀它也不认，所以手拼原样参数
    #[cfg(target_os = "windows")]
    let r = {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer")
            .raw_arg(format!("/select,\"{}\"", plain_path(&file.to_string_lossy())))
            .spawn()
    };
    // xdg-open 没有"选中文件"，打开所在目录
    #[cfg(target_os = "linux")]
    let r = std::process::Command::new("xdg-open").arg(file.parent().unwrap_or(&file)).spawn();
    r.map(|_| ()).map_err(|e| format!("打不开文件夹：{e}"))
}

#[derive(Serialize)]
struct DownloadSettings {
    /// 实际会存到的文件夹
    dir: String,
    /// 是不是用户自己选的（false = 系统「下载」目录）
    custom: bool,
    ask: bool,
    /// 演示模式：下载固定在演示目录，设置改不了
    demo: bool,
}

#[tauri::command]
fn get_download_settings(app: AppHandle) -> Result<DownloadSettings, String> {
    Ok(DownloadSettings {
        dir: downloads_dir(&app)?.display().to_string(),
        custom: !demo::on() && custom_download_dir(&app).is_some(),
        ask: download_ask(&app),
        demo: demo::on(),
    })
}

/// `dir` 为空 = 恢复系统「下载」目录。选文件夹时当场试写一个文件：没权限的话现在就说，
/// 别等到下载时才静默失败（macOS 上选「文稿」「桌面」这类受保护的文件夹，授权框也在这时候弹）
#[tauri::command]
fn set_download_settings(vault: State<Vault>, dir: Option<String>, ask: bool) -> Result<(), String> {
    if demo::on() {
        return Err("演示模式的下载固定存在演示目录里，免得演示时露出真实的文件夹".into());
    }
    let dir = dir.map(|d| d.trim().to_string()).filter(|d| !d.is_empty());
    if let Some(d) = &dir {
        let probe = Path::new(d).join(".switchboard-write-test");
        std::fs::write(&probe, b"").map_err(|e| format!("这个文件夹存不进文件：{e}"))?;
        let _ = std::fs::remove_file(probe);
    }
    with_conn(&vault, |c| {
        db::set_meta(c, DOWNLOAD_DIR_KEY, dir.as_deref().unwrap_or(""))?;
        db::set_meta(c, DOWNLOAD_ASK_KEY, if ask { "1" } else { "0" })
    })
}

/// 「每次都问」：文件先落在下载文件夹里，用户在保存框里选好位置后挪过去，返回新路径。
///
/// 为什么不在下载开始前问：WebKit 和 WebView2 都要在下载回调里**当场**给出落点（wry 同步回填），
/// 在那儿弹模态框等于在浏览器内核的回调里跑一个嵌套消息循环，两边都不稳。先下后挪，两个平台是同一条路。
/// 保存框自己会问"要不要替换同名文件"，所以这里直接覆盖
#[tauri::command]
fn move_download(from: String, to: String) -> Result<String, String> {
    let (from, to) = (PathBuf::from(from), PathBuf::from(to));
    if !is_our_download(&from) || !from.is_file() {
        return Err("找不到刚下载的那个文件，可能已经被移走".into());
    }
    if from != to {
        move_file(&from, &to)?;
    }
    remember_download(&to);
    Ok(to.display().to_string())
}

/// 同一块盘 rename 一下就好；选了 U 盘、网络盘这类别的盘，rename 会失败，退回复制再删
fn move_file(from: &Path, to: &Path) -> Result<(), String> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to).map_err(|e| format!("存不过去：{e}"))?;
    let _ = std::fs::remove_file(from);
    Ok(())
}

/// 去掉 Windows 的 verbatim 前缀：`\\?\C:\x` → `C:\x`，`\\?\UNC\srv\share` → `\\srv\share`。
/// canonicalize 在 Windows 上总是返回带前缀的写法，而资源管理器这类程序不认
#[cfg_attr(not(windows), allow(dead_code))]
fn plain_path(p: &str) -> String {
    if let Some(rest) = p.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    p.strip_prefix(r"\\?\").unwrap_or(p).to_string()
}

/// 在账号下开新页签打开指定地址（会话页页签条上的 +）。
/// async：建子 webview 不能在主线程上等事件循环，见 session::spawn_tab
#[tauri::command]
async fn open_url_in_session(app: AppHandle, account_id: String, url: String) -> Result<(), String> {
    session::open_url(&app, &account_id, &url)
}

/// 切到某个页签。页签是 `target=_blank` / `window.open` 开出来的，见 docs/BROWSER-COMPAT.md。
#[tauri::command]
fn select_session_tab(app: AppHandle, account_id: String, index: usize) -> Result<(), String> {
    session::select_tab(&app, &account_id, index)
}

/// 关掉某个页签。首个页签关不了——那是账号的主页面，要关走 close_session。
#[tauri::command]
fn close_session_tab(app: AppHandle, account_id: String, index: usize) -> Result<(), String> {
    session::close_tab(&app, &account_id, index)
}

/// App 切到 / 切离「会话」页：账号页面是盖在主窗口上的原生 webview，
/// 不主动藏起来会一直挡着别的界面。
#[tauri::command]
fn set_sessions_visible(app: AppHandle, vault: State<Vault>, visible: bool) -> Result<(), String> {
    // 锁着的时候保活的页面还开着（见 lock_down），前端怎么说都不能露出来盖在锁屏上
    let unlocked = vault.0.lock().map(|g| g.is_some()).unwrap_or(false);
    session::set_visible(&app, visible && unlocked)
}

#[tauri::command]
fn active_session(app: AppHandle) -> String {
    session::active_account(&app)
}

/// 把账号/密码/验证码填进"用户最后点过的那个输入框"。
/// 这是不依赖任何选择器的兜底：自动填充认不出的登录页，点一下框再点这个就行。
///
/// 返回填进去的那个值：注入终究有填不进去的站（canvas 键盘、原生控件、层层 iframe），
/// 前端拿到值顺手放进剪贴板，用户 ⌘V 一下总能进去。
#[tauri::command]
fn fill_focused(
    app: AppHandle,
    vault: State<Vault>,
    account_id: String,
    field: String,
) -> Result<String, String> {
    let account = with_conn(&vault, |c| db::get_account(c, &account_id))?;
    // secret = 只往登录站填（见 session::fill_focused）
    let (value, what, secret) = match field.as_str() {
        "username" => (account.username.clone(), "账号".to_string(), false),
        "password" => (account.credential.clone(), "密码".to_string(), true),
        "totp" => (totp::generate(&account.totp_secret)?.code, "验证码".to_string(), true),
        other if other.starts_with("extra:") => {
            let index = other[6..].parse::<usize>().map_err(|_| "附加字段编号无效".to_string())?;
            let extra = account.extra_fields.get(index).ok_or("附加字段不存在")?;
            (extra.value.clone(), extra.label.clone(), extra.secret)
        }
        other => return Err(format!("不认识的字段：{other}")),
    };
    if value.is_empty() {
        return Err(format!("这个账号没填{what}"));
    }
    session::fill_focused(&app, &account_id, &value, &what, secret.then_some(account.login_url.as_str()))?;
    Ok(value)
}

/// 把账号页面当前停的地址存成它的「登录直达 URL」。
/// 腾讯云子用户登录这种藏在二级入口里的页面，手动找 URL 太烦，导航过去点一下就行。
#[tauri::command]
fn pin_current_url(app: AppHandle, vault: State<Vault>, account_id: String) -> Result<String, String> {
    let url = session::current_url(&app, &account_id)?;
    let mut account = with_conn(&vault, |c| db::get_account(c, &account_id))?;
    account.login_url = url.clone();
    with_conn(&vault, |c| db::save_account(c, &account))?;
    Ok(url)
}

// ---------------------------------------------------------------- 平台适配

#[tauri::command]
fn list_platform_configs(vault: State<Vault>) -> Result<Vec<PlatformConfig>, String> {
    with_conn(&vault, db::list_platform_configs)
}

#[tauri::command]
fn save_platform_config(vault: State<Vault>, mut config: PlatformConfig) -> Result<(), String> {
    config.updated_at = now_string();
    with_conn(&vault, |c| db::save_platform_config(c, &config))
}

/// 删一条平台配置。只删没人用的：删了还有账号在用的，那些账号就不再自动填充；
/// 内置平台每次解锁都会被补回来，删了也白删。要清理的是「其他 · 旧主机」这类早就没账号的行
#[tauri::command]
fn delete_platform_config(vault: State<Vault>, platform: String) -> Result<(), String> {
    if db::is_builtin_platform(&platform) {
        return Err(format!("「{platform}」是内置平台，不能删；选择器不对的话改掉或重新指认就行"));
    }
    with_conn(&vault, |c| {
        let n = db::list_accounts(c)?
            .iter()
            .filter(|a| a.platform == platform || config_key(a) == platform)
            .count();
        if n > 0 {
            return Err(format!("还有 {n} 个账号在用「{platform}」，先把它们改到别的平台或删掉再来"));
        }
        db::delete_platform_config(c, &platform)
    })
}

/// 把当前所有平台配置导出成 JSON，便于分享/备份/拿去问 AI 改。
#[tauri::command]
fn export_platform_configs(vault: State<Vault>) -> Result<String, String> {
    let list = with_conn(&vault, db::list_platform_configs)?;
    serde_json::to_string_pretty(&list).map_err(|e| e.to_string())
}

/// 导入平台配置（在线适配的落点）：不用重新打包，把 AI 生成的 JSON 贴进来就生效。
/// 接受单个对象或数组；trigger_event/verified 可以省略。导入的一律标为已验证——
/// 是用户自己确认过要用的。
#[tauri::command]
fn import_platform_configs(vault: State<Vault>, json: String) -> Result<usize, String> {
    let text = json.trim();
    if text.is_empty() {
        return Err("没有内容".into());
    }
    let parsed: Vec<PlatformConfig> = match serde_json::from_str::<Vec<PlatformConfig>>(text) {
        Ok(v) => v,
        // 允许只贴一个对象
        Err(_) => vec![serde_json::from_str::<PlatformConfig>(text)
            .map_err(|e| format!("JSON 格式不对：{e}"))?],
    };
    if parsed.is_empty() {
        return Err("JSON 里没有任何平台配置".into());
    }
    // 先把每一条都校验干净再动库：中途报错会留下半套配置，比整条不导入更难排查
    let now = now_string();
    let mut ready = Vec::with_capacity(parsed.len());
    for mut c in parsed {
        c.platform = c.platform.trim().to_string();
        if c.platform.is_empty() {
            return Err("每条配置都必须有 platform".into());
        }
        if c.trigger_event != "skip"
            && (c.username_selector.trim().is_empty() || c.password_selector.trim().is_empty())
        {
            return Err(format!("「{}」缺少账号或密码输入框选择器", c.platform));
        }
        c.verified = true;
        c.updated_at = now.clone();
        ready.push(c);
    }
    let n = ready.len();
    with_conn(&vault, |conn| {
        for c in &ready {
            db::save_platform_config(conn, c)?;
        }
        Ok(())
    })?;
    Ok(n)
}

/// 进入/退出"手动指认输入框"模式。
#[tauri::command]
fn set_pick_mode(app: AppHandle, account_id: String, on: bool) -> Result<(), String> {
    session::set_picking(&app, &account_id, on)
}

/// 由登录页里的取点脚本回调。只接受当前确实处在指认模式的那个窗口的回传，
/// 别的页面就算猜到命令名也写不进配置。
#[tauri::command]
fn report_picked_selectors(
    app: AppHandle,
    webview: Webview,
    vault: State<Vault>,
    username_selector: String,
    password_selector: String,
) -> Result<(), String> {
    let account_id = session::picking_account(&app, webview.label()).ok_or("当前窗口不在指认模式")?;
    let account = with_conn(&vault, |c| db::get_account(c, &account_id))?;
    let cfg = PlatformConfig {
        platform: config_key(&account),
        username_selector,
        password_selector,
        trigger_event: "input+change".into(),
        verified: true,
        updated_at: now_string(),
    };
    with_conn(&vault, |c| db::save_platform_config(c, &cfg))?;
    session::set_picking(&app, &account_id, false)?;
    // 配置刚写好，立刻按新选择器填一次，省得用户再点一遍"一键登录"
    let _ = session::eval_active(
        &app,
        &account_id,
        &adapter::fill_js(&cfg, &account.username, &account.credential),
    );
    app.emit_to(tauri::EventTarget::labeled("main"), "selectors-picked", cfg.platform)
        .map_err(|e| e.to_string())
}

/// 账号页面加载完后自己报回来的"这页到底能不能用"。
///
/// 后端判不了：wry 没有导航失败回调，而服务器返回 4xx/5xx 时导航其实是**成功**的，
/// 超时判定根本轮不到。所以让页面自己看一眼 HTTP 状态码和正文，回传给这里。
#[tauri::command]
fn report_page_state(app: AppHandle, webview: Webview, status: u16, text_len: u32, interactive: bool) {
    session::mark_failure(&app, webview.label(), classify_page(status, text_len, interactive));
}

/// 页面回传的三个信号 → 要不要判定"打不开"。`None` = 这页能用。
///
/// `status` 为 0 表示拿不到状态码（老 WebKit 没有 `responseStatus`），
/// 这时只能退回看内容。判空要求**又没字又没有任何可交互元素**，两条都满足才算白板——
/// 只看字数会把扫码登录页那种"一张图 + 几个按钮"的页面误杀。
fn classify_page(status: u16, text_len: u32, interactive: bool) -> Option<String> {
    if status >= 400 {
        return Some(match status {
            403 => "服务器拒绝访问（403），多半是限了来源 IP".to_string(),
            404 => "地址不存在（404），检查一下登录直达 URL".to_string(),
            502 | 503 | 504 => format!("服务器没响应（{status}），站点可能没在跑"),
            s => format!("服务器返回 {s}"),
        });
    }
    if text_len == 0 && !interactive {
        return Some("页面是空白的，没有任何内容".to_string());
    }
    None // 正常页面：顺手把上一次的失败结论清掉（刷新成功后要能自己恢复）
}

// ---------------------------------------------------------------- TOTP

/// 从 otpauth:// 链接、二维码图片文件、或剪贴板里的二维码截图取 TOTP 密钥。
/// `text` 优先，其次 `path`，都没有就读剪贴板。osascript 要跑一两百毫秒，别卡主线程
#[tauri::command]
async fn totp_secret_from(text: Option<String>, path: Option<String>) -> Result<String, String> {
    let text = match text {
        Some(t) => t,
        None => tauri::async_runtime::spawn_blocking(move || totp::read_qr(path.as_deref()))
            .await
            .map_err(|e| e.to_string())??,
    };
    totp::secret_from_uri(&text)
}

/// 表单里还没保存的密钥也要能当场出码，好跟手机上的对一下，一致就说明密钥对了
#[tauri::command]
fn totp_preview(secret: String) -> Result<totp::TotpCode, String> {
    totp::generate(&secret)
}

#[tauri::command]
fn totp_code(vault: State<Vault>, account_id: String) -> Result<totp::TotpCode, String> {
    let account = with_conn(&vault, |c| db::get_account(c, &account_id))?;
    totp::generate(&account.totp_secret)
}

// ---------------------------------------------------------------- 备份

/// 导出：`VACUUM INTO` 产出一份用同一把主密码加密的、一致的副本，
/// 不需要停机拷文件，也不需要再加密一层。
#[tauri::command]
fn export_backup(app: AppHandle, vault: State<Vault>) -> Result<String, String> {
    let dir = downloads_dir(&app).or_else(|_| data_dir(&app))?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = now_string().replace([' ', ':'], "-");
    let target = dir.join(format!("switchboard-backup-{stamp}.db"));
    if target.exists() {
        std::fs::remove_file(&target).map_err(|e| e.to_string())?;
    }
    with_conn(&vault, |c| db::export_backup(c, &target))?;
    Ok(target.display().to_string())
}

/// 导入：先用用户给的密码验证这份备份打得开，再整份替换当前库。
/// 替换后当前库的主密码就是这份备份的密码，所以直接锁回登录页。
///
/// 替换当前库跟"清空账号库"是同一级别的整库操作，所以同样要当前主密码
/// ——否则 wipe_vault 那道门槛，拿任意一份自己知道密码的备份导进来就绕过去了。
#[tauri::command]
fn import_backup(
    app: AppHandle,
    vault: State<Vault>,
    path: String,
    password: String,
    current: String,
) -> Result<usize, String> {
    let src = PathBuf::from(&path);
    if !src.exists() {
        return Err("备份文件不存在".into());
    }
    let dst = db_path(&app)?;
    // 库不在时不能拿去验：SQLite 会就地造一个空库，任何密码都"验证通过"
    if !dst.exists() {
        return Err("本机还没有账号库".into());
    }
    drop(db::open_for_unlock(&dst, &current).map_err(|_| "当前主密码不正确".to_string())?);
    // 只读着验：db::open 会顺手建表、写默认配置，等于改了用户选的那份备份文件
    let (count, owner) = {
        let probe = db::open_for_unlock(&src, &password)?;
        (db::count_accounts(&probe)?, db::get_meta(&probe, "username").ok())
    };

    lock_down(&app, &vault, false)?; // 先放开当前库的句柄再覆盖

    // 归档名只到分钟，一分钟内导两次会把第一次留下的原库盖掉——那份才是真正的原库
    let stamp = now_string().replace([' ', ':'], "-");
    let mut replaced = dst.with_extension(format!("db.replaced-{stamp}"));
    for n in 2.. {
        if !replaced.exists() {
            break;
        }
        replaced = dst.with_extension(format!("db.replaced-{stamp}-{n}"));
    }
    swap_in_backup(&src, &dst, &replaced)?;
    prune_archives(&data_dir(&app)?, "replaced");
    // 备份可能是另一个人的库，登录页显示的身份要跟着换——等真换成功了再改
    if let Some(name) = owner {
        write_owner(&app, &name);
    }
    Ok(count)
}

/// 旧库归档每种只留最近这么多份
const KEEP_ARCHIVES: usize = 3;

/// 清掉多余的旧库归档：导入备份留下的 `.replaced-*`、清空账号库留下的 `.locked-*`。
/// 它们用的是**当时**的主密码加密，那把可能比现在的弱，一直堆着等于给离线破解多留靶子。
/// 文件名里的时间戳定长、按字典序就是时间先后，留最新的 `KEEP_ARCHIVES` 份
fn prune_archives(dir: &std::path::Path, kind: &str) {
    let prefix = format!("{DB_FILE}.{kind}-");
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with(&prefix))
        .collect();
    names.sort_unstable_by(|a, b| b.cmp(a));
    for n in names.into_iter().skip(KEEP_ARCHIVES) {
        let _ = std::fs::remove_file(dir.join(n));
    }
}

/// 当前库改名成 `replaced`，把备份复制到 `dst`。
/// 复制失败要把原库挪回来：不然当前库已经改名、新库又没放进去，
/// 下次打开是"创建主账号"，看起来就像数据全丢了
fn swap_in_backup(src: &std::path::Path, dst: &std::path::Path, replaced: &std::path::Path) -> Result<(), String> {
    std::fs::rename(dst, replaced).map_err(|e| e.to_string())?;
    if let Err(e) = std::fs::copy(src, dst) {
        let _ = std::fs::remove_file(dst);
        return Err(match std::fs::rename(replaced, dst) {
            Ok(()) => format!("导入失败，原账号库已恢复：{e}"),
            Err(_) => format!("导入失败：{e}。原账号库改名留在了 {}", replaced.display()),
        });
    }
    Ok(())
}

// ---------------------------------------------------------------- 入口

pub fn run() {
    tauri::Builder::default()
        // 只允许开一个实例。第二次启动时不新开窗口，把已经在跑的那个提到前台——
        // 多开会有两个进程同时抓同一个 SQLCipher 库和同一份 WebKit 数据仓，
        // 轻则 cookie 快照互相覆盖，重则写坏库。
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(Vault::default())
        .manage(session::Sessions::default())
        .manage(RegionCache::default())
        .manage(RegionGuard::default())
        .manage(RegionGrace::default())
        .manage(RegionWatch::default())
        .manage(Activity::default())
        .setup(|app| {
            let handle = app.handle().clone();
            if cfg!(debug_assertions) {
                if let Some(w) = handle.get_webview_window("main") {
                    let _ = w.set_title("Switchboard（开发版 · 独立数据目录）");
                }
            }
            // 主窗口改大小时要重排盖在上面的账号页面
            session::watch_main_window(&handle);

            // 定时抄一份 cookie。**不能只靠退出事件**：macOS 上 Cmd+Q 走的是
            // NSApplication terminate:，不经过"最后一个窗口被销毁"，
            // RunEvent::ExitRequested 根本不会触发；强杀和崩溃就更不会。
            // 有这条兜底，最坏也只丢最近两分钟的登录态。
            // 开发版 60 秒一次（日志里能看见心跳），正式版 120 秒
            let period = if cfg!(debug_assertions) { 60 } else { 120 };
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(period));
                if cfg!(debug_assertions) {
                    eprintln!("[switchboard] 快照定时器心跳");
                }
                snapshot_cookies(&handle, None);
                // 保活也挂在这条线程上：窗口最小化、切到别的 App、停在账号列表页都照跑
                keep_alive_tick(&handle);
            });

            // 区域哨兵：网络是会中途变的（VPN 自动重连、分流规则切换、换 Wi-Fi），
            // 只在开页面那一下查一次，等于只管了开门那一瞬间。所以常驻轮询，
            // 探到境外就按当前模式实时处置（block 直接切断，warn 交前端问）。
            let watch = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    let settings = watch.state::<RegionWatch>();
                    let secs = settings.secs.load(Ordering::Relaxed);
                    tokio::select! {
                        _ = tokio::time::sleep(std::time::Duration::from_secs(secs)) => {},
                        _ = settings.changed.notified() => continue,
                    }
                    region_patrol(&watch).await;
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            is_initialized,
            register,
            unlock,
            current_owner,
            lock,
            get_auto_lock_secs,
            set_auto_lock_secs,
            touch_activity,
            idle_secs,
            change_master_password,
            verify_master_password,
            reset_hint,
            wipe_vault,
            list_accounts,
            get_account,
            save_account,
            delete_account,
            switch_or_open_account,
            check_region,
            patrol_region,
            cut_sessions,
            recheck_and_restore,
            set_region_mode,
            set_region_watch_secs,
            set_region_endpoints,
            reset_region_endpoints,
            list_active_sessions,
            set_keep_alive,
            close_session,
            clear_login,
            reload_session,
            set_sessions_visible,
            select_session_tab,
            tab_history,
            find_in_page,
            open_url_in_session,
            open_in_system_browser,
            reveal_download,
            get_download_settings,
            set_download_settings,
            move_download,
            write_clipboard,
            clear_clipboard_later,
            enter_demo,
            set_demo_tour,
            start_recording,
            stop_recording,
            close_session_tab,
            active_session,
            fill_focused,
            pin_current_url,
            report_page_state,
            list_platform_configs,
            export_platform_configs,
            import_platform_configs,
            save_platform_config,
            delete_platform_config,
            set_pick_mode,
            report_picked_selectors,
            totp_code,
            totp_secret_from,
            totp_preview,
            export_backup,
            import_backup
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // 退出前把会话 cookie 抄进加密库。ExitRequested 时事件循环还活着，
            // wry 的 cookies() 要靠它泵消息才能拿到结果（最多等 1 秒就超时放弃）。
            // 两个退出事件都接：关窗口走 ExitRequested，其余路径尽量在 Exit 补一次
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                snapshot_cookies(app, None);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::{classify_page, config_key, decide, is_our_download, masked, move_download, plain_path, remember_download, prune_archives, ps_quote, sh_quote, swap_in_backup, unmask, Act, ActivityClock, MASK};

    /// 另存只认本 App 下载过的文件；挪过去之后新路径也认（「在访达中显示」要用）
    #[test]
    fn move_download_only_takes_our_own_files() {
        let dir = std::env::temp_dir().join(format!("sb-mv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (ours, stranger, to) = (dir.join("账单.csv"), dir.join("别人的.txt"), dir.join("存到这.csv"));
        std::fs::write(&ours, "a,b").unwrap();
        std::fs::write(&stranger, "x").unwrap();
        remember_download(&ours);

        let s = |p: &std::path::Path| p.display().to_string();
        assert!(move_download(s(&stranger), s(&to)).is_err(), "不是我们下载的文件不能挪");
        assert_eq!(move_download(s(&ours), s(&to)).unwrap(), s(&to));
        assert!(!ours.exists() && std::fs::read_to_string(&to).unwrap() == "a,b");
        assert!(is_our_download(&to), "挪过去的新路径要能在访达里显示");
        assert!(move_download(s(&ours), s(&to)).is_err(), "原文件已经挪走了");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Windows 专用的两个字符串处理，在哪个平台都能测
    #[test]
    fn windows_path_and_quote_helpers() {
        assert_eq!(plain_path(r"\\?\C:\Users\a b\x.csv"), r"C:\Users\a b\x.csv");
        assert_eq!(plain_path(r"\\?\UNC\srv\share\x"), r"\\srv\share\x");
        assert_eq!(plain_path(r"C:\plain"), r"C:\plain", "没前缀的原样返回");
        assert_eq!(ps_quote(r"C:\Users\O'Neil\$x"), r"'C:\Users\O''Neil\$x'", "单引号要双写，$ 不能展开");
    }

    /// 列表打码、存回时换回原值；新账号带着打码的值要拒，不能把圆点当密码存进去
    #[test]
    fn masked_accounts_never_overwrite_real_secrets() {
        let path = std::env::temp_dir().join(format!("sb-mask-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let conn = crate::db::open(&path, "pass 123").unwrap();
        let real: crate::models::Account = serde_json::from_value(serde_json::json!({
            "id": "a1", "platform": "阿里云", "owner_type": "", "related_app": "", "account_role": "",
            "login_url": "", "username": "u", "credential": "真密码", "totp_secret": "JBSWY3DPEHPK3PXP",
            "backup_contact": "", "remark": "",
            "extra_fields": [{"label": "AK", "value": "ak-secret", "secret": true}, {"label": "邮箱", "value": "a@b.c", "secret": false}]
        }))
        .unwrap();
        crate::db::save_account(&conn, &real).unwrap();

        let mut shown = masked(real.clone());
        assert_eq!((shown.credential.as_str(), shown.totp_secret.as_str()), (MASK, MASK));
        assert_eq!(shown.extra_fields[0].value, MASK, "敏感附加字段要打码");
        assert_eq!(shown.extra_fields[1].value, "a@b.c", "不敏感的照常显示");

        shown.remark = "改了备注".into();
        unmask(&conn, &mut shown).unwrap();
        assert_eq!(shown.credential, "真密码");
        assert_eq!(shown.extra_fields[0].value, "ak-secret");

        let mut stranger = masked(real);
        stranger.id = "new-id".into();
        assert!(unmask(&conn, &mut stranger).is_err(), "新账号没有原值可换，必须拒");
        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn only_the_newest_archives_are_kept() {
        let dir = std::env::temp_dir().join(format!("sb-arch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let names = [
            "switchboard.db.replaced-2026-09-01-10-00",
            "switchboard.db.replaced-2026-09-02-10-00",
            "switchboard.db.replaced-2026-09-02-10-00-2",
            "switchboard.db.replaced-2026-09-03-10-00",
            "switchboard.db.locked-1700000000",
            "switchboard.db",
        ];
        for n in names {
            std::fs::write(dir.join(n), b"x").unwrap();
        }
        prune_archives(&dir, "replaced");
        let left = |n: &str| dir.join(n).exists();
        assert!(!left("switchboard.db.replaced-2026-09-01-10-00"), "最旧的该删");
        assert!(left("switchboard.db.replaced-2026-09-02-10-00") && left("switchboard.db.replaced-2026-09-02-10-00-2"));
        assert!(left("switchboard.db.replaced-2026-09-03-10-00"));
        assert!(left("switchboard.db.locked-1700000000") && left("switchboard.db"), "别的文件一个都不能碰");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 「其他」底下的站各存各的选择器，别的平台照旧按平台名共用
    #[test]
    fn other_platform_configs_are_per_site() {
        let mut a: crate::models::Account = serde_json::from_value(serde_json::json!({
            "id": "x", "platform": "其他", "owner_type": "", "related_app": "", "account_role": "",
            "login_url": "http://1.2.3.4:8888/login", "username": "", "credential": "", "totp_secret": "",
            "backup_contact": "", "remark": ""
        }))
        .unwrap();
        assert_eq!(config_key(&a), "其他 · 1.2.3.4:8888");
        a.login_url = "https://log.example.com/".into();
        assert_eq!(config_key(&a), "其他 · log.example.com");
        a.login_url = "".into();
        assert_eq!(config_key(&a), "其他", "没填地址就退回「其他」本身");
        a.platform = "阿里云".into();
        a.login_url = "https://signin.aliyun.com/".into();
        assert_eq!(config_key(&a), "阿里云");
    }

    /// 账号页面是**外部源**，能不能回传全看 capabilities/session.json 里那几个 URL 模式。
    /// 端口是最容易漏的一格：`https://*/*` 的 port 分量是空串，URLPattern 只认默认端口，
    /// 于是 Graylog:9000、宝塔:8888 这类自建服务会被静默拒掉——正好是这个 App 的主力场景。
    #[test]
    fn session_capability_matches_real_login_urls() {
        let cap: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/session.json")).unwrap();
        let pats: Vec<tauri_utils::acl::RemoteUrlPattern> = cap["remote"]["urls"]
            .as_array()
            .expect("session capability 必须声明 remote.urls")
            .iter()
            .map(|p| p.as_str().unwrap().parse().unwrap())
            .collect();
        for url in [
            "https://gitee.com/login",
            "http://1.2.3.4:8888/login",       // 宝塔
            "https://log.example.com:9000/",   // Graylog
            "https://console.aliyun.com/?x=1#/login",
            "http://127.0.0.1:65138/aliyun/login", // 演示模式的本机模拟页
        ] {
            let u: url::Url = url.parse().unwrap();
            assert!(
                pats.iter().any(|p| p.test(&u)),
                "{url} 不在 session capability 的 remote.urls 里，这页的指认/加载回传会被 ACL 拒掉",
            );
        }
    }

    /// ACL 和命令表必须同步。`permissions/` 目录一存在，Tauri 就对**所有** App 命令开启
    /// 校验，漏掉一条的后果是"点到那个按钮才发现被拒"——正是 report_picked_selectors
    /// 那次的坑。这里把 generate_handler! 的清单跟权限文件对一遍，漏了当场红。
    #[test]
    fn every_command_is_covered_by_a_permission() {
        let src = include_str!("lib.rs");
        let list = src
            .split_once("generate_handler![")
            .expect("找不到 generate_handler!")
            .1
            .split_once("])")
            .expect("命令清单没有收尾")
            .0;
        let acl = concat!(
            include_str!("../permissions/main-window.toml"),
            include_str!("../permissions/session-report.toml"),
        );
        for cmd in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            assert!(
                acl.contains(&format!("\"{cmd}\"")),
                "命令 {cmd} 没写进 permissions/，主窗口或登录页调它会被 ACL 拒掉",
            );
        }
    }

    /// 哨兵的处置矩阵。这张表错一格，后果是"该断的没断"或者"每分钟弹一次框"，
    /// 两种都得实际跑上几十分钟才看得出来，所以在这儿钉死。
    #[test]
    fn block_mode_cuts_once_and_only_once() {
        assert_eq!(decide("block", true, false, true, false), Act::Cut, "探到境外必须立刻断");
        assert_eq!(decide("block", true, false, false, false), Act::Cut, "哪怕不是刚翻转（比如开机时就在境外）也要断");
        assert_eq!(decide("block", true, true, false, false), Act::Nothing, "已经断了就别每分钟重断一次");
    }

    #[test]
    fn warn_mode_asks_only_on_the_flip() {
        assert_eq!(decide("warn", true, false, true, false), Act::Ask, "刚变成境外，问一次");
        assert_eq!(decide("warn", true, false, false, false), Act::Nothing, "一直在境外待着，不要反复骚扰");
        assert_eq!(decide("warn", true, true, true, false), Act::Nothing, "已经断了就没什么可问的了");
    }

    /// 「仍要继续」之后的宽限期：不断、不问；但网络恢复照样接回被断的页面
    #[test]
    fn grace_after_continue_suppresses_cut_and_ask() {
        assert_eq!(decide("block", true, false, false, true), Act::Nothing, "刚放行的页面不能转头就断");
        assert_eq!(decide("warn", true, false, true, true), Act::Nothing, "刚说了继续，别紧接着再问");
        assert_eq!(decide("block", false, true, true, true), Act::Restore, "宽限期里网络好了也得接回来");
    }

    /// 回到大陆必须无条件接回来。漏了这条，用户关掉 VPN 之后会永远卡在空白页，
    /// 而且看不出是被谁断的——这是最难自查的一种坏法。
    #[test]
    fn coming_back_always_restores() {
        assert_eq!(decide("block", false, true, true, false), Act::Restore);
        assert_eq!(decide("warn", false, true, false, false), Act::Restore, "warn 模式断的也要接回来");
        assert_eq!(decide("block", false, false, true, false), Act::Nothing, "本来就没断，不用管");
    }

    /// 用户报的那个场景：限了 IP 的宝塔面板。
    /// 实测 **69 毫秒**就返回 404（nginx 默认错误页，146 字节）——导航是**成功**的，
    /// 所以超时判定永远不会触发，只能靠状态码抓。这条钉死的就是那个场景。
    #[test]
    fn fast_http_error_is_a_failure_not_a_successful_load() {
        let r = classify_page(404, 19, false).expect("404 必须判成打不开");
        assert!(r.contains("404"), "原因里要有状态码，用户才知道是什么问题：{r}");
        assert!(classify_page(403, 0, false).unwrap().contains("IP"), "403 要点出限 IP 这个最常见的原因");
        assert!(classify_page(502, 0, false).is_some());
        assert!(classify_page(500, 9999, true).is_some(), "有内容也没用，5xx 就是错误页");
    }

    /// 反过来：正常页面绝不能被误杀，否则真能用的站会被藏起来变成错误提示。
    #[test]
    fn working_pages_are_never_flagged() {
        assert!(classify_page(200, 1200, true).is_none(), "普通登录页");
        assert!(classify_page(200, 0, true).is_none(), "扫码登录页：一张图加几个按钮，一个字都没有");
        assert!(classify_page(0, 800, true).is_none(), "拿不到状态码时，有内容就放行");
        assert!(classify_page(302, 400, true).is_none(), "3xx 不算错误状态码");
    }

    /// 状态码拿不到（老 WebKit 没有 responseStatus）时的兜底：彻底空白才算失败。
    #[test]
    fn blank_page_needs_both_signals() {
        assert!(classify_page(0, 0, false).is_some(), "又没字又没元素 = 真白板");
        assert!(classify_page(0, 0, true).is_none(), "只是没文字，但有元素——可能是还没渲染完的 SPA");
        assert!(classify_page(0, 5, false).is_none(), "有字就不算白板");
    }

    fn clock(limit: u64, mono_ago: u64, wall_ago: u64) -> ActivityClock {
        use std::time::{Duration, Instant, SystemTime};
        ActivityClock {
            mono: Instant::now().checked_sub(Duration::from_secs(mono_ago)),
            wall: SystemTime::now().checked_sub(Duration::from_secs(wall_ago)),
            limit,
        }
    }

    /// 合上盖子睡了两小时：单调钟（macOS 上睡眠不走）只过了几秒，墙上钟过了两小时。
    /// 自动锁定必须按两小时算——这正是"离开电脑"最常见的样子
    #[test]
    fn sleep_counts_as_idle() {
        let c = clock(300, 5, 7200);
        assert!(c.idle_secs() >= 7200);
        assert!(c.expired());
    }

    /// 往回拨了系统时间（墙上钟变成"未来"）不能把空闲时长清零
    #[test]
    fn clock_set_backwards_still_counts() {
        // 单调钟只往回推 90 秒：推太多的话刚开机的机器上 checked_sub 会给 None
        let mut c = clock(60, 90, 0);
        c.wall = Some(std::time::SystemTime::now() + std::time::Duration::from_secs(3600));
        assert!(c.idle_secs() >= 90);
        assert!(c.expired());
    }

    /// 已经超时的操作不能续命：人回来随手一点，时钟不能被刷新，得让它锁
    #[test]
    fn expired_clock_is_not_refreshed_by_a_touch() {
        let mut c = clock(300, 400, 400);
        assert!(c.touch(), "超时后的操作要报告超时");
        assert!(c.idle_secs() >= 400, "超时后的操作把时钟刷新了，自动锁定会被一次点击绕过");

        let mut fresh = clock(300, 100, 100);
        assert!(!fresh.touch(), "没超时的正常续命");
        assert!(fresh.idle_secs() < 5);
    }

    #[test]
    fn auto_lock_off_never_expires() {
        assert!(!clock(0, 99_999, 99_999).expired());
        assert_eq!(ActivityClock::default().idle_secs(), 0, "还没记过时按刚动过算");
    }

    /// 导入时复制备份失败（备份被删了、盘满了）：原库必须原样回到原位，
    /// 不能落得"旧库改了名、新库没进来"，下次打开变成"创建主账号"
    #[test]
    fn failed_import_puts_the_original_vault_back() {
        let dir = std::env::temp_dir().join(format!("sb-swap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (dst, replaced) = (dir.join("switchboard.db"), dir.join("switchboard.db.replaced-x"));
        std::fs::write(&dst, b"original vault").unwrap();

        let err = swap_in_backup(&dir.join("gone.db"), &dst, &replaced).unwrap_err();
        assert!(err.contains("已恢复"), "{err}");
        assert_eq!(std::fs::read(&dst).unwrap(), b"original vault");
        assert!(!replaced.exists());

        let backup = dir.join("backup.db");
        std::fs::write(&backup, b"backup vault").unwrap();
        swap_in_backup(&backup, &dst, &replaced).unwrap();
        assert_eq!(std::fs::read(&dst).unwrap(), b"backup vault");
        assert_eq!(std::fs::read(&replaced).unwrap(), b"original vault", "原库改名留着，不是删掉");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 粘进终端的重置命令：路径里的 `$` 反引号 单引号 空格都得原样当字面量
    #[test]
    fn reset_command_paths_are_shell_safe() {
        assert_eq!(sh_quote("/a b/$HOME/`id`/it's"), r"'/a b/$HOME/`id`/it'\''s'");
    }
}
