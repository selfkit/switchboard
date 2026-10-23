//! 凭据默认只投递到相同 origin；已知登录跳转逐个列出，不猜测域名归属。
pub const GROUPS: &str = include_str!("credential-origins.json");

pub fn allowed_origins(login: &tauri::Url) -> Vec<String> {
    if !matches!(login.scheme(), "http" | "https") {
        return Vec::new();
    }
    let origin = login.origin().ascii_serialization();
    let groups: Vec<Vec<String>> = serde_json::from_str(GROUPS).expect("credential origin groups");
    groups
        .into_iter()
        .find(|g| g.contains(&origin))
        .unwrap_or_else(|| vec![origin])
}

pub fn allows(login: &tauri::Url, page: &tauri::Url) -> bool {
    matches!(page.scheme(), "http" | "https")
        && allowed_origins(login).contains(&page.origin().ascii_serialization())
}

pub fn script() -> String {
    include_str!("origin-policy.js").replace("__SB_ORIGIN_GROUPS__", GROUPS)
}

/// 页面可能在原生校验与执行之间导航，再在执行处核一次不可伪造的 location.origin。
pub fn guard(login: &tauri::Url, js: &str) -> String {
    let checks = allowed_origins(login)
        .iter()
        .map(|origin| {
            format!(
                "location.origin === {}",
                serde_json::to_string(origin).expect("origin JSON")
            )
        })
        .collect::<Vec<_>>()
        .join(" || ");
    let checks = if checks.is_empty() {
        "false".to_string()
    } else {
        checks
    };
    format!("if ({checks}) {{ {js} }}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_require_exact_origin_or_reviewed_login_origin() {
        let ok = |a: &str, b: &str| allows(&a.parse().unwrap(), &b.parse().unwrap());
        assert!(ok(
            "https://signin.aliyun.com/a",
            "https://passport.aliyun.com/b"
        ));
        assert!(ok("http://localhost:8000/a", "http://localhost:8000/b"));
        for (a, b) in [
            ("https://a.github.io", "https://b.github.io"),
            ("https://a.example.com", "https://b.example.com"),
            ("https://signin.aliyun.com", "https://arbitrary.aliyun.com"),
            ("https://signin.aliyun.com", "http://signin.aliyun.com"),
            (
                "https://signin.aliyun.com",
                "https://signin.aliyun.com:8443",
            ),
            ("http://localhost:8000", "http://localhost:9000"),
            ("file:///a", "file:///b"),
        ] {
            assert!(!ok(a, b), "{a} -> {b}");
        }
    }
}
