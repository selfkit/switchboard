//! 网络区域检测。完整设计见 docs/NETWORK-GUARD.md。
//!
//! 一句话：打开账号页面前，判断**阿里云会看到的出口**在不在中国大陆。
//!
//! 两个信号，缺一不可：
//! 1. **出口归属**——问国内的 IP 归属服务"我从哪来"
//! 2. **外网可达性**——能不能直接访问 Google。大陆的普通网络访问不了，
//!    能通就说明有隧道在跑（分流代理 / TUN / VPN）
//!
//! ⚠️ **实施红线**：探测用的 HTTP client **绝对不能** `.no_proxy()`，
//! 也不能关掉 reqwest 的 `system-proxy` feature。
//! 内嵌 WebView 走 macOS 系统代理，探测必须走同一条路——探测看到什么出口，
//! 阿里云就会看到什么出口。一旦绕过代理，用户开了全局 VPN 也会被判成"国内"，
//! 这道闸就成了摆设，而且**失效得无声无息**。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

/// 港澳台。国内 IP 库习惯把它们挂在 `country = "中国"` 底下、用省级字段区分，
/// 所以**只判 country 会把这三个全放进来**——实测：
///   香港 219.76.10.1 → 中国,香港,  ／ 澳门 202.175.3.8 → 中国,澳门,
///   台湾 168.95.1.1  → 中国,台湾,
/// 这份名单是硬条件，不是保险措施。
const DENY_REGIONS: &[&str] = &[
    "香港", "香港特别行政区", "澳门", "澳門", "澳门特别行政区", "台湾", "台灣", "臺灣", "台湾省",
    "Hong Kong", "Macao", "Macau", "Taiwan",
];

/// 表示"中国"的各种写法。注意：ISO 码单独精确匹配，不走 contains，
/// 免得某个地名里恰好含 "CN" 被误判成大陆。
const MAINLAND_NAMES: &[&str] = &["中国", "中國", "China"];

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Origin {
    /// 中国大陆，放行
    Mainland,
    /// 境外（含港澳台），按模式处理
    Outside { country: String, region: String },
    /// 探测失败。**任何模式下都不拦截**——测不到通常是断网/服务挂了，
    /// 不是"用户在境外"，因为测不到就把人锁在门外是把工具变成障碍。
    Unknown { reason: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct Verdict {
    pub origin: Origin,
    /// 外网是否直接可达 = 是否存在隧道
    pub tunnel: bool,
    /// 系统是否配置了代理
    pub proxy: bool,
    /// 探测到的出口 IP，仅用于展示让用户自己核对
    pub ip: String,
}

impl Verdict {
    pub fn blocked(&self) -> bool {
        match self.origin {
            Origin::Outside { .. } => true,
            // 国内探针走直连、外网探针却可达时，目标站点可能被代理规则分流。
            // 无法证明目标站点也走大陆出口，按当前区域检测模式处理。
            Origin::Mainland => self.tunnel,
            Origin::Unknown { .. } => false,
        }
    }
}

/// 一个探测端点。存在 `meta` 表里可改——端点挂了、改了格式，改配置就行，不用发版。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Endpoint {
    pub name: String,
    pub url: String,
    /// 点号路径，数字段表示数组下标，如 `data.location.0`
    pub ip_path: String,
    pub country_path: String,
    pub region_path: String,
}

pub fn default_endpoints() -> Vec<Endpoint> {
    vec![
        // 实测 302ms，最快。国内直连。
        Endpoint {
            name: "qq".into(),
            url: "https://r.inews.qq.com/api/ip2city".into(),
            ip_path: "ip".into(),
            country_path: "country".into(),
            region_path: "province".into(),
        },
        // 实测 1244ms。返回 ["中国","重庆","重庆","","联通"]
        Endpoint {
            name: "ipip".into(),
            url: "https://myip.ipip.net/json".into(),
            ip_path: "data.ip".into(),
            country_path: "data.location.0".into(),
            region_path: "data.location.1".into(),
        },
    ]
}

/// 外网可达性探针。返回空 204，最轻量。
const TUNNEL_PROBE: &str = "https://www.google.com/generate_204";

/// 按点号路径取值，数字段当数组下标。
fn pick<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = v;
    for seg in path.split('.') {
        cur = match seg.parse::<usize>() {
            Ok(i) => cur.get(i)?,
            Err(_) => cur.get(seg)?,
        };
    }
    Some(cur)
}

fn as_text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// 纯判定：给国家和省级字段，判断是不是中国大陆。不联网，可单测。
pub fn judge(country: &str, region: &str) -> Origin {
    let c = country.trim();
    let r = region.trim();

    if c.is_empty() {
        return Origin::Unknown {
            reason: "数据源没返回国家字段".into(),
        };
    }

    // 港澳台可能出现在任一字段：国内库放 region（中国,香港），
    // 走 ISO 码的数据源放 country（HK/TW/MO）。两边都查。
    let hit_deny = |s: &str| DENY_REGIONS.iter().any(|d| s.contains(d));
    if hit_deny(c) || hit_deny(r) {
        return Origin::Outside {
            country: c.to_string(),
            region: r.to_string(),
        };
    }

    // ISO 码精确匹配
    match c.to_ascii_uppercase().as_str() {
        "CN" => return Origin::Mainland,
        "HK" | "MO" | "TW" => {
            return Origin::Outside {
                country: c.to_string(),
                region: r.to_string(),
            }
        }
        _ => {}
    }

    if MAINLAND_NAMES.iter().any(|m| c.contains(m)) {
        return Origin::Mainland;
    }

    Origin::Outside {
        country: c.to_string(),
        region: r.to_string(),
    }
}

/// 各端点的结果合并。**不一致时取更保守的那个**——宁可多问一次，不可漏放一次。
/// 没拿到结果的端点（`None`）不参与，全都没拿到才算 Unknown。
pub fn merge_all(results: impl IntoIterator<Item = Option<(String, Origin)>>) -> (String, Origin) {
    let pick_worse = |x: (String, Origin), y: (String, Origin)| -> (String, Origin) {
        // Outside > Unknown > Mainland（越靠前越保守）
        let rank = |o: &Origin| match o {
            Origin::Outside { .. } => 2,
            Origin::Unknown { .. } => 1,
            Origin::Mainland => 0,
        };
        if rank(&y.1) > rank(&x.1) {
            y
        } else {
            x
        }
    };
    results.into_iter().flatten().reduce(pick_worse).unwrap_or_else(|| {
        (
            String::new(),
            Origin::Unknown {
                reason: "所有探测端点都不可达".into(),
            },
        )
    })
}

/// 探测用的 client。
///
/// ⚠️ 这里**故意不调 `.no_proxy()`**，也不要给 reqwest 关掉 `system-proxy` feature。
/// 见本文件顶部的红线说明。
fn client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(timeout)
        .user_agent("Switchboard")
        .build()
        .map_err(|e| e.to_string())
}

async fn probe_one(ep: &Endpoint) -> Option<(String, Origin)> {
    let c = client(Duration::from_secs(3)).ok()?;
    let text = c.get(&ep.url).send().await.ok()?.text().await.ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let country = as_text(pick(&v, &ep.country_path));
    let region = as_text(pick(&v, &ep.region_path));
    let ip = as_text(pick(&v, &ep.ip_path));
    Some((ip, judge(&country, &region)))
}

/// 外网能不能直接访问。通 = 有隧道在跑。
/// **测不通不是失败，测不通本身就是结果**（说明是大陆网络）。
async fn probe_tunnel() -> bool {
    let Ok(c) = client(Duration::from_secs(2)) else {
        return false;
    };
    matches!(c.get(TUNNEL_PROBE).send().await, Ok(r) if r.status().as_u16() == 204)
}

/// 系统有没有配代理。只作提示用，不参与放行判断。
fn system_proxy_configured() -> bool {
    if ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"]
        .iter()
        .any(|k| std::env::var(k).map(|v| !v.is_empty()).unwrap_or(false))
    {
        return true;
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("scutil").arg("--proxy").output() {
            let s = String::from_utf8_lossy(&out.stdout);
            return s.contains("HTTPEnable : 1") || s.contains("HTTPSEnable : 1");
        }
    }
    // Windows 的系统代理在注册表里（跟 reqwest 读的是同一处，所以探测本身不受影响，这里只管提示）。
    // 输出里值那一行形如 `    ProxyEnable    REG_DWORD    0x1`，跟系统语言无关
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        if let Ok(out) = std::process::Command::new("reg")
            .args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings", "/v", "ProxyEnable"])
            .creation_flags(crate::CREATE_NO_WINDOW)
            .output()
        {
            return String::from_utf8_lossy(&out.stdout).split_whitespace().last() == Some("0x1");
        }
    }
    false
}

/// 开发期伪装地区，否则这功能没法验收——总不能真飞一趟香港。
/// release 构建里这个开关不存在，避免变成现成的绕过手段。
fn fake_override() -> Option<Verdict> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let v = std::env::var("SWITCHBOARD_FAKE_REGION").ok()?;
    let origin = match v.to_ascii_uppercase().as_str() {
        "CN" | "CN_TUNNEL" => Origin::Mainland,
        "NONE" => Origin::Unknown {
            reason: "伪装：探测失败".into(),
        },
        other => Origin::Outside {
            country: other.to_string(),
            region: String::new(),
        },
    };
    Some(Verdict {
        origin,
        tunnel: !v.eq_ignore_ascii_case("CN"),
        proxy: true,
        ip: "0.0.0.0（伪装）".into(),
    })
}

pub async fn check(endpoints: &[Endpoint]) -> Verdict {
    if let Some(f) = fake_override() {
        return f;
    }
    // scutil 是个子进程，别让异步线程干等它
    let proxy = tokio::task::spawn_blocking(system_proxy_configured).await.unwrap_or(false);

    // 配了几个端点就探几个（设置页能存任意多个，以前只用前两个、其余悄悄不管），
    // 跟外网探针一起并发，总耗时按最慢的算（上限 3 秒）
    let mut probes = tokio::task::JoinSet::new();
    for ep in endpoints {
        let ep = ep.clone();
        probes.spawn(async move { probe_one(&ep).await });
    }
    let (tunnel, results) = tokio::join!(probe_tunnel(), probes.join_all());

    let (ip, origin) = merge_all(results);
    Verdict {
        origin,
        tunnel,
        proxy,
        ip,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outside(o: &Origin) -> bool {
        matches!(o, Origin::Outside { .. })
    }

    /// 这组数据全部来自真实探测（见 docs/NETWORK-GUARD.md §2），不是编的。
    /// 港澳台那三条是整个功能最容易写错的地方：它们的 country 都是"中国"。
    #[test]
    fn hong_kong_macao_taiwan_are_not_mainland() {
        assert_eq!(judge("中国", "江苏"), Origin::Mainland, "南京应放行");
        assert_eq!(judge("中国", "浙江"), Origin::Mainland, "杭州应放行");
        assert_eq!(judge("中国", "重庆市"), Origin::Mainland, "重庆应放行");

        assert!(outside(&judge("中国", "香港")), "香港必须拦——它的 country 也是「中国」");
        assert!(outside(&judge("中国", "澳门")), "澳门必须拦");
        assert!(outside(&judge("中国", "台湾")), "台湾必须拦");
        assert!(outside(&judge("中国", "台湾省")), "台湾省必须拦");
        assert!(outside(&judge("中国", "澳門")), "繁体澳門必须拦");
    }

    #[test]
    fn iso_codes_are_matched_exactly() {
        assert_eq!(judge("CN", ""), Origin::Mainland);
        assert!(outside(&judge("HK", "")));
        assert!(outside(&judge("MO", "")));
        assert!(outside(&judge("TW", "")));
        // 别因为地名里含 "cn" 就误判成大陆
        assert!(outside(&judge("Cncalia", "")));
    }

    #[test]
    fn foreign_countries_are_outside() {
        assert!(outside(&judge("美国", "加利福尼亚州")));
        assert!(outside(&judge("日本", "东京都")));
        assert!(outside(&judge("荷兰", "")));
    }

    #[test]
    fn missing_country_is_unknown_not_outside() {
        // 解析失败绝不能当成"非中国"，否则一次接口抽风就把人拦在门外
        assert!(matches!(judge("", "重庆"), Origin::Unknown { .. }));
        assert!(matches!(judge("   ", ""), Origin::Unknown { .. }));
    }

    #[test]
    fn merge_takes_the_more_conservative_one() {
        let cn = || ("1.1.1.1".to_string(), Origin::Mainland);
        let hk = || {
            (
                "2.2.2.2".to_string(),
                Origin::Outside {
                    country: "中国".into(),
                    region: "香港".into(),
                },
            )
        };
        let unk = || {
            (
                String::new(),
                Origin::Unknown {
                    reason: "x".into(),
                },
            )
        };

        // 一个说大陆一个说境外 → 按境外
        assert!(outside(&merge_all([Some(cn()), Some(hk())]).1));
        assert!(outside(&merge_all([Some(hk()), Some(cn())]).1));
        // 一个成功一个失败 → 用成功的那个
        assert_eq!(merge_all([Some(cn()), None]).1, Origin::Mainland);
        assert!(outside(&merge_all([None, Some(hk())]).1));
        // 未知比大陆保守，但没境外保守
        assert!(matches!(merge_all([Some(cn()), Some(unk())]).1, Origin::Unknown { .. }));
        assert!(outside(&merge_all([Some(unk()), Some(hk())]).1));
        // 全挂 → 未知
        assert!(matches!(merge_all([None, None]).1, Origin::Unknown { .. }));
        // 第三个端点探到境外也得算数
        assert!(outside(&merge_all([Some(cn()), Some(cn()), Some(hk())]).1));
        assert_eq!(merge_all([None, Some(cn()), None]).1, Origin::Mainland, "没拿到结果的端点不参与");
    }

    /// 点号路径要能取到实测响应里的字段
    #[test]
    fn dot_path_reads_real_responses() {
        // qq 的真实返回
        let qq: Value = serde_json::from_str(
            r#"{"ret":0,"ip":"203.0.113.79","country":"中国","province":"重庆市","city":"重庆市"}"#,
        )
        .unwrap();
        assert_eq!(as_text(pick(&qq, "ip")), "203.0.113.79");
        assert_eq!(as_text(pick(&qq, "country")), "中国");
        assert_eq!(as_text(pick(&qq, "province")), "重庆市");

        // ipip 的真实返回，位置是数组
        let ipip: Value = serde_json::from_str(
            r#"{"ret":"ok","data":{"ip":"203.0.113.79","location":["中国","重庆","重庆","","联通"]}}"#,
        )
        .unwrap();
        assert_eq!(as_text(pick(&ipip, "data.ip")), "203.0.113.79");
        assert_eq!(as_text(pick(&ipip, "data.location.0")), "中国");
        assert_eq!(as_text(pick(&ipip, "data.location.1")), "重庆");

        // 路径不存在 → 空串 → judge 会判成 Unknown，而不是 Outside
        assert_eq!(as_text(pick(&qq, "data.nope")), "");
        assert!(matches!(
            judge(&as_text(pick(&qq, "nope")), ""),
            Origin::Unknown { .. }
        ));
    }

    /// 伪装开关必须真的生效，否则没法验收——总不能真飞一趟香港。
    /// 这条同时锁住"未知不拦截"这个最关键的安全阀。
    #[test]
    fn fake_override_drives_every_branch() {
        for (fake, should_block) in [
            ("CN", false),
            ("CN_TUNNEL", true),
            ("HK", true),
            ("TW", true),
            ("MO", true),
            ("US", true),
            ("NONE", false), // 探测失败 → 未知 → 绝不拦
        ] {
            std::env::set_var("SWITCHBOARD_FAKE_REGION", fake);
            let v = fake_override().expect("debug 构建下伪装开关必须生效");
            assert_eq!(
                v.blocked(),
                should_block,
                "SWITCHBOARD_FAKE_REGION={fake} 的拦截行为不对"
            );
        }
        std::env::remove_var("SWITCHBOARD_FAKE_REGION");
        assert!(fake_override().is_none(), "没设环境变量时不能伪装");
    }

    /// 真联网的冒烟测试，默认跳过（`cargo test -- --ignored` 才跑）。
    /// 单测能保证判定逻辑对，但保证不了"端点还活着、字段还是那个字段"。
    #[tokio::test]
    #[ignore]
    async fn live_probe_smoke() {
        std::env::remove_var("SWITCHBOARD_FAKE_REGION");
        let v = check(&default_endpoints()).await;
        println!("出口 IP   : {}", v.ip);
        println!("归属      : {:?}", v.origin);
        println!("外网可达  : {}（true = 有隧道在跑）", v.tunnel);
        println!("系统代理  : {}", v.proxy);
        assert!(
            !matches!(v.origin, Origin::Unknown { .. }),
            "两个端点都没返回可用结果，检查网络或端点是否失效"
        );
    }

    #[test]
    fn verdict_blocks_outside_and_mainland_with_tunnel() {
        let mk = |o: Origin, tunnel: bool| Verdict {
            origin: o,
            tunnel,
            proxy: true,
            ip: String::new(),
        };
        assert!(!mk(Origin::Mainland, false).blocked());
        assert!(mk(Origin::Mainland, true).blocked(), "大陆出口与可达的境外探针并存时要处理");
        assert!(!mk(Origin::Unknown { reason: "x".into() }, true).blocked(), "未知绝不拦");
        assert!(mk(Origin::Outside {
            country: "日本".into(),
            region: String::new()
        }, false)
        .blocked());
    }
}
