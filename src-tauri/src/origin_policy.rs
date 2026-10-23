//! 同一可注册域名的登录子域自动兼容；PSL 的 PRIVATE 部分隔离不同托管租户。
use publicsuffix::{List, Psl};
use std::sync::OnceLock;

const DATA: &str = include_str!("../resources/public_suffix_list.dat");

fn list() -> &'static List {
    static LIST: OnceLock<List> = OnceLock::new();
    LIST.get_or_init(|| DATA.parse().expect("bundled ASCII public suffix list"))
}

fn registered_domain(host: &str) -> Option<String> {
    let host = host.strip_suffix('.').unwrap_or(host);
    if host.split('.').any(str::is_empty) {
        return None;
    }
    let domain = list().domain(host.as_bytes())?;
    std::str::from_utf8(domain.as_bytes())
        .ok()
        .map(str::to_owned)
}

pub fn allows(login: &tauri::Url, page: &tauri::Url) -> bool {
    if !matches!(login.scheme(), "http" | "https") || login.scheme() != page.scheme() {
        return false;
    }
    if login.origin() == page.origin() {
        return true;
    }
    // IP、localhost 等没有可注册域名，维持精确来源匹配，避免串到其他本地服务。
    match (login.domain(), page.domain()) {
        (Some(a), Some(b)) => match (registered_domain(a), registered_domain(b)) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        },
        _ => false,
    }
}

pub fn script() -> String {
    static SCRIPT: OnceLock<String> = OnceLock::new();
    SCRIPT
        .get_or_init(|| {
            let rules: Vec<_> = DATA
                .lines()
                .filter_map(|line| line.split_whitespace().next())
                .filter(|line| !line.starts_with("//"))
                .collect();
            include_str!("origin-policy.js").replace(
                "__SB_PUBLIC_SUFFIX_RULES__",
                &serde_json::to_string(&rules).expect("PSL JSON"),
            )
        })
        .clone()
}

/// 原生侧先核验同站，再只投递到本次核验的实际来源，防止校验后跨站导航。
/// 不调用可被网页覆盖的辅助函数或原型方法来判断凭据投递目标。
pub fn guard(page: &tauri::Url, js: &str) -> String {
    if !matches!(page.scheme(), "http" | "https") {
        return String::new();
    }
    let origin = serde_json::to_string(&page.origin().ascii_serialization()).expect("origin JSON");
    format!("if (location.origin === {origin}) {{ {js} }}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_login_subdomains_work_without_an_allowlist() {
        let ok = |a: &str, b: &str| allows(&a.parse().unwrap(), &b.parse().unwrap());
        for (a, b) in [
            (
                "https://admin.example.com/",
                "https://login.example.com/auth",
            ),
            ("https://signin.aliyun.com/", "https://passport.aliyun.com/"),
            (
                "https://admin.company.com.cn/",
                "https://login.company.com.cn/",
            ),
            (
                "https://admin.example.co.uk/",
                "https://auth.example.co.uk/",
            ),
            (
                "https://admin.example.com:8443/",
                "https://login.example.com:9443/",
            ),
            ("https://admin.example.com./", "https://login.example.com/"),
            (
                "https://admin.食狮.公司.cn/",
                "https://login.xn--85x722f.xn--55qx5d.cn/",
            ),
        ] {
            assert!(ok(a, b), "{a} -> {b}");
        }
    }

    #[test]
    fn shared_hosting_tenants_and_unrelated_services_stay_separate() {
        let ok = |a: &str, b: &str| allows(&a.parse().unwrap(), &b.parse().unwrap());
        for (a, b) in [
            ("https://a.github.io/", "https://b.github.io/"),
            ("https://a.appspot.com/", "https://b.appspot.com/"),
            ("https://a.vercel.app/", "https://b.vercel.app/"),
            ("https://a.co.uk/", "https://b.co.uk/"),
            ("https://a.example.com/", "https://b.example.net/"),
            ("https://signin.aliyun.com/", "http://signin.aliyun.com/"),
            ("http://localhost:8000/", "http://localhost:9000/"),
            ("http://127.0.0.1:8000/", "http://127.0.0.1:9000/"),
            ("http://[::1]:8000/", "http://[::1]:9000/"),
            ("file:///a", "file:///b"),
        ] {
            assert!(!ok(a, b), "{a} -> {b}");
        }
        assert!(ok("http://localhost:8000/a", "http://localhost:8000/b"));
        assert!(ok(
            "https://auth.a.github.io/",
            "https://login.a.github.io/"
        ));
    }

    #[test]
    fn bundled_list_passes_upstream_public_suffix_vectors() {
        let vectors: Vec<(Option<String>, Option<String>)> = serde_json::from_str(include_str!(
            "../../scripts/fixtures/public-suffix-cases.json"
        ))
        .unwrap();
        for (host, expected) in vectors {
            let actual = host
                .as_ref()
                .and_then(|h| tauri::Url::parse(&format!("https://{h}/")).ok())
                .and_then(|u| u.domain().and_then(registered_domain));
            let expected = expected
                .and_then(|h| tauri::Url::parse(&format!("https://{h}/")).ok())
                .and_then(|u| u.domain().map(str::to_owned));
            assert_eq!(actual, expected, "host: {host:?}");
        }
    }
}
