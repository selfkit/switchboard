//! 保活会话存在期间阻止空闲睡眠；仍允许锁屏、熄屏和用户主动睡眠。
//! 系统电源 API 在主线程上成对调用，不能把活动永久泄漏到进程退出。

use std::cell::RefCell;
use tauri::AppHandle;

thread_local! {
    static ACTIVITY: RefCell<Option<platform::Activity>> = const { RefCell::new(None) };
}

/// 打开、关闭会话或切换保活后立即同步，不等下一轮刷新。
/// 在主线程里重新读取会话，避免并发开关时较旧的状态覆盖较新的状态。
pub fn sync(app: &AppHandle) {
    let handle = app.clone();
    if let Err(e) = app.run_on_main_thread(move || {
        let needed = !crate::session::keep_alive_ids(&handle).is_empty();
        ACTIVITY.with(|slot| {
            if let Err(e) = update(&mut slot.borrow_mut(), needed, platform::Activity::begin) {
                eprintln!("[switchboard] 保活电源设置失败：{e}");
            }
        });
    }) {
        eprintln!("[switchboard] 保活电源设置未派发：{e}");
    }
}

fn update<T>(slot: &mut Option<T>, needed: bool, begin: impl FnOnce() -> Result<T, String>) -> Result<(), String> {
    if needed && slot.is_none() {
        *slot = Some(begin()?);
    } else if !needed {
        *slot = None;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
mod platform {
    use objc2::{rc::Retained, runtime::ProtocolObject};
    use objc2_foundation::{NSActivityOptions, NSObjectProtocol, NSProcessInfo, NSString};

    pub struct Activity(Retained<ProtocolObject<dyn NSObjectProtocol>>);

    impl Activity {
        pub fn begin() -> Result<Self, String> {
            Ok(Self(NSProcessInfo::processInfo().beginActivityWithOptions_reason(
                // 包含 IdleSystemSleepDisabled，不包含 IdleDisplaySleepDisabled。
                NSActivityOptions::UserInitiated,
                &NSString::from_str("保活：锁屏后继续刷新账号页面"),
            )))
        }
    }

    impl Drop for Activity {
        fn drop(&mut self) {
            // SAFETY: token 来自 beginActivity，且只在这里结束一次。
            unsafe { NSProcessInfo::processInfo().endActivity(&self.0) };
        }
    }
}

#[cfg(windows)]
mod platform {
    // https://learn.microsoft.com/windows/win32/api/winbase/nf-winbase-setthreadexecutionstate
    #[link(name = "kernel32")]
    extern "system" {
        fn SetThreadExecutionState(flags: u32) -> u32;
    }
    const ES_CONTINUOUS: u32 = 0x80000000;
    const ES_SYSTEM_REQUIRED: u32 = 0x00000001;

    pub struct Activity;

    impl Activity {
        pub fn begin() -> Result<Self, String> {
            // SAFETY: 只传文档规定的标志；获取和释放都在 Tauri 主线程上。
            if unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) } == 0 {
                return Err("无法阻止系统空闲睡眠".into());
            }
            Ok(Self)
        }
    }

    impl Drop for Activity {
        fn drop(&mut self) {
            // SAFETY: 清掉同一线程先前设置的要求，不请求保持屏幕亮着。
            unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    pub struct Activity;
    impl Activity {
        pub fn begin() -> Result<Self, String> { Ok(Self) }
    }
}

#[cfg(test)]
mod tests {
    use super::update;
    use std::{cell::Cell, rc::Rc};

    struct Activity(Rc<Cell<usize>>);
    impl Drop for Activity {
        fn drop(&mut self) { self.0.set(self.0.get() + 1); }
    }

    #[test]
    fn activity_lasts_until_the_last_keep_alive_session_closes() {
        let starts = Cell::new(0);
        let ends = Rc::new(Cell::new(0));
        let begin = || {
            starts.set(starts.get() + 1);
            Ok(Activity(ends.clone()))
        };
        let mut slot = None;
        // 无会话 → 开一个 → 再开一个 → 锁库保留 → 关一个 → 关最后一个。
        for needed in [false, true, true, true, true] {
            update(&mut slot, needed, begin).unwrap();
        }
        assert_eq!(starts.get(), 1);
        assert_eq!(ends.get(), 0);
        update(&mut slot, false, begin).unwrap();
        update(&mut slot, false, begin).unwrap();
        assert_eq!(ends.get(), 1);
        update(&mut slot, true, begin).unwrap();
        assert_eq!(starts.get(), 2, "再次开启保活必须重新申请");
        update(&mut slot, false, begin).unwrap();
        assert_eq!(ends.get(), 2);
    }

    #[test]
    fn failed_acquisition_can_be_retried() {
        let mut slot = None;
        assert!(update(&mut slot, true, || Err::<(), _>("failed".into())).is_err());
        assert!(slot.is_none());
        update(&mut slot, true, || Ok(())).unwrap();
        assert!(slot.is_some());
    }
}
