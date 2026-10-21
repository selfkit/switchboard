//! 一个很小的滚动日志。Windows 上的问题（刷新卡死、保活第二天失效）现场只有用户那一台机器，
//! 没有日志就只能靠猜。只记事件，不记任何账号凭据、密码、cookie。
//!
//! 位置：`<数据目录>/switchboard.log`，超过 2 MB 滚到 `switchboard.log.1`（只留一份）。
//! 写失败就静默——日志不该影响主流程。

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use time::format_description::well_known::Rfc3339;

/// 单个文件的上限。再大就没有用户愿意发过来了。
const MAX_BYTES: u64 = 2 * 1024 * 1024;

static PATH: OnceLock<PathBuf> = OnceLock::new();
/// 串行化写入，避免保活线程、区域哨兵、前端命令同时写时把行写花
static LOCK: Mutex<()> = Mutex::new(());

/// 在 App 启动、数据目录确定之后调用一次。重复调用只有第一次生效。
pub fn init(dir: &Path) {
    let path = dir.join("switchboard.log");
    let _ = PATH.set(path);
}

/// 日志文件在哪。设置页/诊断信息用它告诉用户从哪拿。
pub fn path() -> Option<PathBuf> {
    PATH.get().cloned()
}

/// 写一条。`kind` 是短标签（keep-alive / region / webview / panic / app），方便 grep。
pub fn write(kind: &str, message: &str) {
    let Some(path) = PATH.get() else { return };
    let stamp = time::OffsetDateTime::now_local()
        .unwrap_or_else(|_| time::OffsetDateTime::now_utc())
        .format(&Rfc3339)
        .unwrap_or_default();
    let Ok(_guard) = LOCK.lock() else { return };
    write_locked(path, &stamp, kind, message);
}

fn write_locked(path: &Path, stamp: &str, kind: &str, message: &str) {
    rotate_if_needed(path);
    let line = format!("{stamp} [{kind}] {message}\n");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(line.as_bytes());
    }
}

fn rotate_if_needed(path: &Path) {
    let big = std::fs::metadata(path).map(|m| m.len() >= MAX_BYTES).unwrap_or(false);
    if !big {
        return;
    }
    let old = path.with_file_name("switchboard.log.1");
    let _ = std::fs::remove_file(&old);
    let _ = std::fs::rename(path, &old);
}

/// panic 也要留下现场。release 是 `panic = "abort"`，hook 会先跑，然后再 abort。
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "非字符串 panic".into()
        };
        write("panic", &format!("{location} {payload}"));
        default(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_rotates_without_leaving_two_big_files() {
        let dir = std::env::temp_dir().join(format!("sb-log-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        // 不走全局 PATH：单测是并行跑的，全局只能 init 一次
        let path = dir.join("switchboard.log");
        write_locked(&path, "2026-01-02T03:04:05+08:00", "test", "hello");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[test] hello"), "要写进去: {text}");
        assert!(text.contains('T'), "时间戳要可读: {text}");

        // 直接撑大，下一次写入应该滚动
        let mut big = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        std::io::Write::write_all(&mut big, &vec![b'x'; MAX_BYTES as usize + 1]).unwrap();
        drop(big);
        write_locked(&path, "2026-01-02T03:04:06+08:00", "test", "after-rotate");
        let rotated = path.with_file_name("switchboard.log.1");
        assert!(rotated.exists(), "超过上限要滚到 .1");
        assert!(
            std::fs::metadata(&path).unwrap().len() < MAX_BYTES,
            "滚动后当前文件不能还是满的"
        );
        assert!(std::fs::read_to_string(&path).unwrap().contains("after-rotate"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
