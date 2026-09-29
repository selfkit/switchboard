//! 演示模式：一套假账号 + 本机模拟的登录页，给别人看功能用。
//!
//! 三条保证，缺一条演示就可能露馅：
//! - **不碰真实账号库**：数据目录换成 `demo/`（见 lib.rs 的 `data_dir`），每次进来整个清掉重建
//! - **不连外网**：登录页由本机 127.0.0.1 上的一个小服务现做，区域检测在演示库里默认关
//!   （关 = 一个探测请求都不发）
//! - **下载不落进真实「下载」目录**：点"在访达中显示"时不会把真实文件列表露给观众
//!
//! 模拟页的输入框属性照着 db.rs 里各平台的默认选择器做，所以演示的是**真的**自动填充链路，
//! 不是专门给演示开的后门。锁定（手动或自动）即退出演示，回到真实账号库的登录页。

use crate::models::{Account, ExtraField};
use percent_encoding::{percent_decode_str, utf8_percent_encode, NON_ALPHANUMERIC};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

static ON: AtomicBool = AtomicBool::new(false);
/// 一键自动演示进行中：模拟登录页填好就自己点登录，不用人去点
static TOUR: AtomicBool = AtomicBool::new(false);
/// 正在录的屏：screencapture 进程 + 输出文件
static REC: Mutex<Option<(Child, PathBuf)>> = Mutex::new(None);
static PORT: OnceLock<u16> = OnceLock::new();
/// 模拟登录态的凭据。每次进演示模式换一个：macOS 的会话数据仓按账号 id 存在系统目录里，
/// 清 demo/ 清不到它，不换的话上一轮登进去的 cookie 还有效，演示一开场就是"已登录"
static TOKEN: Mutex<String> = Mutex::new(String::new());

/// 演示库的主密码。复制明文、改设置时要输，所以顶栏上直接写出来
pub const PASSWORD: &str = "demo";
pub const OWNER: &str = "演示用户";

pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

pub fn set(on: bool) {
    ON.store(on, Ordering::Relaxed);
    if !on {
        TOUR.store(false, Ordering::Relaxed);
    }
}

pub fn set_tour(on: bool) {
    TOUR.store(on, Ordering::Relaxed);
}

/// 一个模拟平台。`user` / `pass` 是输入框的属性，**必须命中 db.rs 里该平台的默认选择器**
struct Plat {
    slug: &'static str,
    name: &'static str,
    color: &'static str,
    user: &'static str,
    pass: &'static str,
}

const PLATS: &[Plat] = &[
    Plat { slug: "aliyun", name: "阿里云", color: "#ff6a00",
        user: r#"id="loginName" name="loginName" placeholder="账号名/邮箱/手机号""#,
        pass: r#"id="loginPassword" name="loginPassword" type="password" placeholder="登录密码""# },
    Plat { slug: "tencent", name: "腾讯云", color: "#006eff",
        user: r#"name="email" placeholder="请输入邮箱地址""#,
        pass: r#"name="password" type="password" placeholder="请输入账号密码""# },
    Plat { slug: "yunzhanghu", name: "云账户", color: "#2b5cff",
        user: r#"name="manage_name" placeholder="管理员账号""#,
        pass: r#"name="password" type="password" placeholder="密码""# },
    Plat { slug: "zego", name: "即构", color: "#1a53ff",
        user: r#"name="user" placeholder="请输入邮箱/手机号""#,
        pass: r#"name="password" type="password" placeholder="请输入密码""# },
    Plat { slug: "wechat", name: "微信小程序", color: "#07c160",
        user: r#"name="account" placeholder="邮箱/微信号""#,
        pass: r#"name="password" type="password" placeholder="密码""# },
];

/// 演示账号。id 固定：macOS 的会话数据仓按 id 派生，随机 id 每演示一次就在系统目录里多留一份
pub fn accounts(port: u16) -> Vec<Account> {
    // (序号, 平台, 归属, 关联应用, 角色, 账号, 密码, 带 TOTP, 备注)
    const ROWS: &[(u8, &str, &str, &str, &str, &str, &str, bool, &str)] = &[
        (1, "aliyun", "星河科技", "官网 · 生产环境", "主账号", "admin@xinghe.example", "Demo-Aliyun-01", true, "月底对账用这个"),
        (2, "aliyun", "星河科技", "官网 · 测试环境", "RAM子账号", "qa@xinghe.example", "Demo-Aliyun-02", false, ""),
        (3, "tencent", "星河科技", "小程序后端", "子账号", "dev@xinghe.example", "Demo-Tencent-01", true, ""),
        (4, "tencent", "蓝鲸传媒", "CDN 与对象存储", "主账号", "web@lanjing.example", "Demo-Tencent-02", false, ""),
        (5, "yunzhanghu", "蓝鲸传媒", "灵活用工结算", "主账号", "lanjing_finance", "Demo-Yzh-01", false, "打款前需复核"),
        (6, "zego", "星河科技", "音视频通话 SDK", "主账号", "rtc@xinghe.example", "Demo-Zego-01", false, ""),
        (7, "wechat", "蓝鲸传媒", "会员小程序", "服务号", "lanjing_mp", "Demo-Wechat-01", false, ""),
        (8, "aliyun", "个人账号", "个人博客", "主账号", "me@personal.example", "Demo-Aliyun-03", true, ""),
    ];
    ROWS.iter()
        .map(|&(n, slug, owner, app, role, user, pwd, totp, remark)| Account {
            id: format!("de300000-0000-4000-8000-{n:012}"),
            platform: PLATS.iter().find(|p| p.slug == slug).map(|p| p.name).unwrap_or("其他").into(),
            owner_type: owner.into(),
            related_app: app.into(),
            account_role: role.into(),
            login_url: format!("http://127.0.0.1:{port}/{slug}/login"),
            username: user.into(),
            credential: pwd.into(),
            totp_secret: if totp { "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP".into() } else { String::new() },
            backup_contact: "演示联系人 13800000000".into(),
            remark: remark.into(),
            extra_fields: if n == 1 {
                vec![
                    ExtraField { label: "企业主体".into(), value: "星河科技有限公司（演示）".into(), secret: false },
                    ExtraField { label: "AccessKey Secret".into(), value: "demo-ak-secret-0000".into(), secret: true },
                ]
            } else {
                vec![]
            },
        })
        .collect()
}

/// 起模拟页服务（整个进程只起一次），并作废上一轮的模拟登录态。返回端口
pub fn serve() -> Result<u16, String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    *TOKEN.lock().map_err(|e| e.to_string())? = format!("{nanos:x}");
    if let Some(p) = PORT.get() {
        return Ok(*p);
    }
    // 只绑回环地址：局域网里别人访问不到
    let l = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("演示页面服务起不来：{e}"))?;
    let port = l.local_addr().map_err(|e| e.to_string())?.port();
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            // 一个连接一个线程：WebKit 会预连接，空连接卡在单线程里的话后面的请求全得排队
            std::thread::spawn(move || {
                let _ = handle(s);
            });
        }
    });
    let _ = PORT.set(port);
    Ok(port)
}

fn handle(mut s: TcpStream) -> std::io::Result<()> {
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut r = BufReader::new(s.try_clone()?);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("/").to_string());
    let (mut len, mut cookie) = (0usize, String::new());
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "content-length" => len = v.trim().parse::<usize>().unwrap_or(0).min(64 * 1024),
                "cookie" => cookie = v.trim().to_string(),
                _ => {}
            }
        }
    }
    let mut body = vec![0; len];
    r.read_exact(&mut body)?;
    s.write_all(&route(&method, &path, &cookie, &String::from_utf8_lossy(&body)))
}

fn route(method: &str, path: &str, cookie: &str, body: &str) -> Vec<u8> {
    let path = path.split('?').next().unwrap_or("");
    let (slug, page) = path.trim_matches('/').split_once('/').unwrap_or((path.trim_matches('/'), ""));
    let Some(p) = PLATS.iter().find(|p| p.slug == slug) else {
        return reply("404 Not Found", &[], "text/plain; charset=utf-8", "演示页面不存在".as_bytes());
    };
    let login = format!("/{slug}/login");
    let console = format!("/{slug}/console");
    let user = logged_in(cookie);
    match (method, page) {
        ("POST", "login") => {
            // 各平台的输入框 name 不一样，按表单顺序取：第一个是账号、第二个是密码
            let vals: Vec<String> = body
                .split('&')
                .map(|kv| kv.split_once('=').map(|(_, v)| v).unwrap_or(""))
                .map(|v| percent_decode_str(&v.replace('+', " ")).decode_utf8_lossy().trim().to_string())
                .collect();
            match vals.as_slice() {
                [u, pwd, ..] if !u.is_empty() && !pwd.is_empty() => {
                    let token = TOKEN.lock().map(|t| t.clone()).unwrap_or_default();
                    let enc = utf8_percent_encode(u, NON_ALPHANUMERIC);
                    // 不带过期时间的会话 cookie，正好演示"关掉再开还是登录态"（cookie 快照）
                    redirect(&console, &format!("sbdemo={token}.{enc}; Path=/; HttpOnly; SameSite=Lax"))
                }
                _ => html(&login_page(p, "请输入账号和密码")),
            }
        }
        (_, "login") if user.is_some() => redirect(&console, ""),
        (_, "login") => html(&login_page(p, "")),
        (_, "logout") => redirect(&login, "sbdemo=; Path=/; Max-Age=0"),
        (_, "bill") if user.is_some() => {
            let name = format!("{}-演示账单.csv", p.name);
            let disp = format!(
                "Content-Disposition: attachment; filename=\"bill.csv\"; filename*=UTF-8''{}",
                utf8_percent_encode(&name, NON_ALPHANUMERIC)
            );
            let csv = format!("\u{feff}日期,项目,金额\n2026-09-01,{} 演示资源,1280.00\n2026-09-15,流量包（演示）,2000.00\n", p.name);
            reply("200 OK", &[disp], "application/octet-stream", csv.as_bytes())
        }
        (_, page @ ("console" | "docs")) => match user {
            Some(u) => html(&console_page(p, &u, page == "docs")),
            None => redirect(&login, ""),
        },
        _ => redirect(&login, ""),
    }
}

/// cookie 里的登录态：`sbdemo=<本轮 token>.<账号>`，token 对不上就当没登录
fn logged_in(cookie: &str) -> Option<String> {
    let v = cookie.split(';').find_map(|c| c.trim().strip_prefix("sbdemo="))?;
    let (token, user) = v.split_once('.')?;
    let cur = TOKEN.lock().ok()?;
    (!cur.is_empty() && token == *cur).then(|| percent_decode_str(user).decode_utf8_lossy().to_string())
}

fn reply(status: &str, headers: &[String], ctype: &str, body: &[u8]) -> Vec<u8> {
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n",
        body.len()
    );
    for h in headers {
        head += h;
        head += "\r\n";
    }
    head += "\r\n";
    let mut out = head.into_bytes();
    out.extend_from_slice(body);
    out
}

fn html(s: &str) -> Vec<u8> {
    reply("200 OK", &[], "text/html; charset=utf-8", s.as_bytes())
}

fn redirect(to: &str, set_cookie: &str) -> Vec<u8> {
    let mut h = vec![format!("Location: {to}")];
    if !set_cookie.is_empty() {
        h.push(format!("Set-Cookie: {set_cookie}"));
    }
    reply("302 Found", &h, "text/plain; charset=utf-8", b"")
}

/// 账号是表单里填进来的，照样得转义
fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

const CSS: &str = r#"*{box-sizing:border-box}body{margin:0;font:14px/1.6 -apple-system,"PingFang SC",sans-serif;background:#f5f6f8;color:#1d2129}
header{display:flex;align-items:center;gap:10px;height:52px;padding:0 24px;color:#fff;font-size:16px;font-weight:600}
header .tag{font-size:11px;font-weight:400;padding:1px 8px;border-radius:99px;background:rgba(255,255,255,.22)}
header .right{margin-left:auto;font-size:13px;font-weight:400}header a{color:#fff;margin-left:12px}
main{max-width:960px;margin:48px auto;padding:0 24px}
.card{background:#fff;border-radius:10px;box-shadow:0 1px 3px rgba(0,0,0,.08);padding:24px}
form.card{max-width:380px;margin:0 auto;display:flex;flex-direction:column;gap:14px}
h1{margin:0;font-size:22px;font-weight:500}label{display:flex;flex-direction:column;gap:6px;font-size:12px;color:#4e5969}
input{padding:10px 12px;border:1px solid #d4d6db;border-radius:7px;font-size:14px}
button{padding:11px;border:0;border-radius:7px;color:#fff;font-size:15px;cursor:pointer}
.err{color:#e34d59;font-size:12px;margin:0}.hint{color:#86909c;font-size:12px;margin:0}
.grid{display:grid;grid-template-columns:repeat(3,1fr);gap:16px;margin:20px 0}.num{font-size:26px;font-weight:600}
.links a{margin-right:20px}"#;

fn shell(p: &Plat, title: &str, right: &str, main: &str) -> String {
    format!(
        r#"<!doctype html><html lang="zh"><head><meta charset="utf-8"><title>{title}</title><style>{CSS}</style></head>
<body><header style="background:{c}">{n}<span class="tag">演示环境</span><span class="right">{right}</span></header>
<main>{main}</main></body></html>"#,
        c = p.color,
        n = p.name,
    )
}

fn login_page(p: &Plat, err: &str) -> String {
    let err = if err.is_empty() { String::new() } else { format!(r#"<p class="err">{err}</p>"#) };
    let main = format!(
        r#"<form class="card" method="post"><h1>登录{n}</h1>{err}
<label>账号<input {u} autocomplete="off"></label>
<label>密码<input {pw}></label>
<button style="background:{c}">登录</button>
<p class="hint">本页由 Switchboard 演示模式在本机生成，不连接真实的{n}</p></form>{auto}"#,
        n = p.name,
        u = p.user,
        pw = p.pass,
        c = p.color,
        // 自动演示时等 App 把两个框填好，停一秒让观众看清，再自己提交
        auto = if TOUR.load(Ordering::Relaxed) {
            "<script>var t=setInterval(function(){var i=document.querySelectorAll('input');\
if(i[0].value&&i[1].value){clearInterval(t);setTimeout(function(){document.forms[0].submit()},1000)}},300)</script>"
        } else {
            ""
        },
    );
    shell(p, &format!("{} · 登录", p.name), "", &main)
}

fn console_page(p: &Plat, user: &str, docs: bool) -> String {
    let right = format!(r#"欢迎，{}<a href="/{}/logout">退出</a>"#, esc(user), p.slug);
    let main = if docs {
        format!(r#"<div class="card"><h1>{} 文档中心</h1><p>这是从控制台用新页签打开的页面，页签条上能看到它。</p></div>"#, p.name)
    } else {
        format!(
            r#"<div class="card"><h1>{n} 控制台概览</h1>
<div class="grid"><div class="card"><div class="hint">运行中的资源</div><div class="num">12</div></div>
<div class="card"><div class="hint">本月费用</div><div class="num">¥3,280.00</div></div>
<div class="card"><div class="hint">待处理告警</div><div class="num">0</div></div></div>
<p class="links"><a href="/{s}/bill" download>下载本月账单</a><a href="/{s}/bill" target="_blank">在新页签下载账单</a><a href="/{s}/docs" target="_blank">在新页签打开文档中心</a></p>
<p class="hint">以上全是演示数据</p></div>"#,
            n = p.name,
            s = p.slug,
        )
    };
    shell(p, &format!("{} · 控制台", p.name), &right, &main)
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

/// 用系统自带的 screencapture 录 `rect`（x,y,w,h，屏幕点坐标）这一块，录满 `secs` 秒它自己收尾写文件。
///
/// 定时录而不是"开始/喊停"：App 中途被 ⌘Q 掉（那条路不走退出事件）也不会留下一个一直在录的进程，
/// 也不用去猜 screencapture 认哪种停止信号。
pub fn start_recording(rect: &str, path: PathBuf, secs: u64) -> Result<(), String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (rect, path, secs);
        return Err("录屏目前只支持 macOS".into());
    }
    #[cfg(target_os = "macos")]
    {
        // 没权限时 screencapture 直接失败或只录到桌面，先问清楚。
        // Request 会弹系统授权框 / 把 App 加进列表，但授权后要重开 App 才生效
        if !unsafe { CGPreflightScreenCaptureAccess() } {
            unsafe { CGRequestScreenCaptureAccess() };
            return Err("需要录屏权限：到「系统设置 → 隐私与安全性 → 录屏与系统录音」里打开 Switchboard，重开 App 后再试".into());
        }
        let mut rec = REC.lock().map_err(|e| e.to_string())?;
        if rec.is_some() {
            return Err("已经在录屏了".into());
        }
        let mut child = std::process::Command::new("screencapture")
            .args(["-x", "-V", &secs.to_string(), "-R", rect])
            .arg(&path)
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("录屏启动不了：{e}"))?;
        // 一启动就退出 = 没录上，把原因拿回来，别让人演完才发现视频是空的
        std::thread::sleep(Duration::from_millis(600));
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            let mut err = String::new();
            if let Some(mut e) = child.stderr.take() {
                let _ = e.read_to_string(&mut err);
            }
            return Err(format!("录屏没开起来（{status}）：{}", err.trim()));
        }
        *rec = Some((child, path));
        Ok(())
    }
}

/// `keep`：等录制按时长收尾，在访达里选中视频；否则是中途停止，掐掉进程、删掉半截文件。
/// 会阻塞到录完（最多就是 start 时给的秒数），调用方别放在主线程上
pub fn stop_recording(keep: bool) -> Result<PathBuf, String> {
    let (mut child, path) = REC.lock().map_err(|e| e.to_string())?.take().ok_or("没有在录屏")?;
    if !keep {
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_file(&path);
        return Ok(path);
    }
    let _ = child.wait();
    if !path.exists() {
        return Err("录屏结束了但没有生成文件，检查一下录屏权限".into());
    }
    // 录屏只有 macOS 有（start_recording 在别的系统上直接报错），这里跟着只在 macOS 上选中文件
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(&path).spawn();
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(path: &str, cookie: &str) -> String {
        String::from_utf8_lossy(&route("GET", path, cookie, "")).to_string()
    }

    /// 每个模拟平台都有账号，而且全指向本机、能出登录表单
    #[test]
    fn every_account_points_at_a_local_login_page() {
        let accts = accounts(1234);
        for p in PLATS {
            assert!(accts.iter().any(|a| a.platform == p.name), "{} 没有演示账号", p.name);
        }
        for a in &accts {
            assert!(a.login_url.starts_with("http://127.0.0.1:1234/"), "演示账号不能指向外网：{}", a.login_url);
            assert!(get(a.login_url.trim_start_matches("http://127.0.0.1:1234"), "").contains("<form"));
        }
    }

    #[test]
    fn login_sets_a_session_that_only_this_round_accepts() {
        serve().unwrap();
        let out = String::from_utf8_lossy(&route("POST", "/aliyun/login", "", "loginName=a%40b.c&loginPassword=x")).to_string();
        assert!(out.starts_with("HTTP/1.1 302"), "{out}");
        let cookie = out.lines().find_map(|l| l.strip_prefix("Set-Cookie: ")).unwrap().split(';').next().unwrap().to_string();
        assert!(get("/aliyun/console", &cookie).contains("欢迎，a@b.c"));
        assert!(get("/aliyun/console", "").starts_with("HTTP/1.1 302"), "没登录不给看控制台");
        // 空密码登不进去
        assert!(!String::from_utf8_lossy(&route("POST", "/aliyun/login", "", "loginName=a&loginPassword=")).starts_with("HTTP/1.1 302"));
        // 账号是表单里来的，页面上要转义
        let token = TOKEN.lock().unwrap().clone();
        let page = get("/aliyun/console", &format!("sbdemo={token}.%3Cscript%3E"));
        assert!(page.contains("&lt;script&gt;") && !page.contains("<script>"));
        // 自动演示时登录页自己提交，平时不
        assert!(!get("/aliyun/login", "").contains("<script>"));
        set_tour(true);
        assert!(get("/aliyun/login", "").contains("forms[0].submit()"));
        set(false);
        assert!(!get("/aliyun/login", "").contains("<script>"), "退出演示要把自动提交一起关掉");
        // 再进一次演示模式，上一轮的 cookie 作废
        serve().unwrap();
        assert!(get("/aliyun/console", &cookie).starts_with("HTTP/1.1 302"));
    }
}
