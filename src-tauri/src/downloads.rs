use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};
use tauri::{AppHandle, Emitter};

/// macOS 完成回调只有 URL。同一 URL 的并行请求必须拒绝，不能猜哪个文件下完了。
#[derive(Default)]
pub struct Tracker {
    paths: HashMap<String, PathBuf>,
}
impl Tracker {
    pub fn contains(&self, url: &str) -> bool {
        self.paths.contains_key(url)
    }
    pub fn begin(&mut self, url: String, path: PathBuf) -> bool {
        if let std::collections::hash_map::Entry::Vacant(entry) = self.paths.entry(url) {
            entry.insert(path);
            true
        } else {
            false
        }
    }
    pub fn finish(&mut self, url: &str, native: Option<PathBuf>) -> Option<PathBuf> {
        self.paths
            .remove(url)
            .map(|chosen| native.unwrap_or(chosen))
    }
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

#[derive(Clone, serde::Serialize)]
pub struct Done {
    pub id: u64,
    pub name: String,
    pub path: String,
    pub success: bool,
    pub ask: bool,
}
static NEXT: AtomicU64 = AtomicU64::new(1);
static PENDING: Mutex<Vec<Done>> = Mutex::new(Vec::new());

pub fn complete(app: &AppHandle, path: PathBuf, success: bool) {
    let done = Done {
        id: NEXT.fetch_add(1, Ordering::Relaxed),
        name: path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        path: path.display().to_string(),
        success,
        ask: crate::download_ask(app),
    };
    if let Ok(mut pending) = PENDING.lock() {
        pending.push(done.clone());
    }
    let _ = app.emit_to(
        tauri::EventTarget::labeled("main"),
        "download-finished",
        done,
    );
}
pub fn pending() -> Vec<Done> {
    PENDING.lock().map(|v| v.clone()).unwrap_or_default()
}
pub fn acknowledge(id: u64) {
    if let Ok(mut pending) = PENDING.lock() {
        pending.retain(|d| d.id != id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overlapping_downloads_finish_with_their_own_paths() {
        let mut tracker = Tracker::default();
        tracker.begin("a".into(), PathBuf::from("a.csv"));
        tracker.begin("b".into(), PathBuf::from("b.csv"));
        assert!(tracker.contains("a"));
        assert!(
            !tracker.begin("a".into(), PathBuf::from("incorrect.csv")),
            "重复 URL 不能覆盖原路径"
        );
        assert_eq!(tracker.finish("a", None), Some(PathBuf::from("a.csv")));
        assert!(!tracker.is_empty());
        assert_eq!(tracker.finish("a", None), None);
        assert_eq!(tracker.finish("b", None), Some(PathBuf::from("b.csv")));
        assert!(tracker.is_empty());
    }
    #[test]
    fn native_completion_path_takes_precedence() {
        let mut tracker = Tracker::default();
        tracker.begin("a".into(), "suggested".into());
        assert_eq!(
            tracker.finish("a", Some("actual".into())),
            Some("actual".into())
        );
    }
}
