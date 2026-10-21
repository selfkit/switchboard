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
    // 电源请求对象（PowerCreateRequest）比 SetThreadExecutionState 在现代待机（S0ix）上更可靠：
    // 前者是"这个进程需要系统保持唤醒"，后者是"这个线程"级别的旧 API，
    // 一些笔记本合盖/熄屏后仍会滑进现代待机。失败时退回旧 API，不让保活因此办不成。
    // https://learn.microsoft.com/windows/win32/power/powercreatepowerrequest
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Power::{
        PowerClearRequest, PowerCreateRequest, PowerSetRequest, PowerRequestSystemRequired,
    };
    use windows::Win32::System::SystemServices::POWER_REQUEST_CONTEXT_VERSION;
    use windows::Win32::System::Threading::{
        POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0,
    };

    pub struct Activity {
        /// 电源请求句柄。None = 退回了旧的 SetThreadExecutionState
        request: Option<HANDLE>,
        /// REASON_CONTEXT 里的指针指向它，必须跟请求活一样久
        _reason: Vec<u16>,
        legacy: bool,
    }

    impl Activity {
        pub fn begin() -> Result<Self, String> {
            let mut reason: Vec<u16> = "Switchboard 保活：锁屏后继续刷新账号页面"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let context = REASON_CONTEXT {
                Version: POWER_REQUEST_CONTEXT_VERSION,
                Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
                Reason: REASON_CONTEXT_0 { SimpleReasonString: PWSTR(reason.as_mut_ptr()) },
            };
            // SAFETY: context 指向 reason，reason 跟着 Activity 一起活；
            // 句柄在 Drop 里先清请求再关闭。
            if let Ok(handle) = unsafe { PowerCreateRequest(&context) } {
                if unsafe { PowerSetRequest(handle, PowerRequestSystemRequired) }.is_ok() {
                    return Ok(Self { request: Some(handle), _reason: reason, legacy: false });
                }
                let _ = unsafe { CloseHandle(handle) };
            }
            let _ = crate::logfile::write("power", "电源请求对象不可用，退回 SetThreadExecutionState");
            // SAFETY: 只传文档规定的标志；获取和释放都在 Tauri 主线程上。
            if unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) } == 0 {
                return Err("无法阻止系统空闲睡眠".into());
            }
            Ok(Self { request: None, _reason: reason, legacy: true })
        }
    }

    impl Drop for Activity {
        fn drop(&mut self) {
            if let Some(handle) = self.request.take() {
                // SAFETY: 句柄来自 PowerCreateRequest，且只在这里关闭一次。
                unsafe {
                    let _ = PowerClearRequest(handle, PowerRequestSystemRequired);
                    let _ = CloseHandle(handle);
                }
                return;
            }
            if self.legacy {
                // SAFETY: 清掉同一线程先前设置的要求，不请求保持屏幕亮着。
                unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
            }
        }
    }

    // 旧 API 只做兜底，所以这里自己声明，不再多拉一份 windows 的 feature。
    #[link(name = "kernel32")]
    extern "system" {
        fn SetThreadExecutionState(flags: u32) -> u32;
    }
    const ES_CONTINUOUS: u32 = 0x8000_0000;
    const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;
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
