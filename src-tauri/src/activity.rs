//! 自动锁定只信任原生输入，不接受远端页面的活动 IPC。
#[cfg(target_os = "macos")]
pub fn install(app: &tauri::AppHandle) {
    use block2::RcBlock;
    use objc2_app_kit::{NSEvent, NSEventMask};
    use std::ptr::NonNull;
    let app = app.clone();
    let handler = RcBlock::new(move |event: NonNull<NSEvent>| {
        crate::native_activity(&app);
        event.as_ptr()
    });
    // SAFETY: AppKit 本地事件在主线程回调；原样返回，监听器与应用同寿命。
    if let Some(monitor) = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(
            NSEventMask::LeftMouseDown
                | NSEventMask::RightMouseDown
                | NSEventMask::KeyDown
                | NSEventMask::ScrollWheel
                | NSEventMask::OtherMouseDown,
            &handler,
        )
    } {
        std::mem::forget(monitor);
    }
}

#[cfg(windows)]
pub fn install(app: &tauri::AppHandle) {
    use tauri::Manager;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let app = app.clone();
    std::thread::spawn(move || {
        let mut last = None;
        loop {
            let mut input = LASTINPUTINFO {
                cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
                ..Default::default()
            };
            // SAFETY: 正确初始化的 Win32 输出结构；仅记录 App 前台期间新发生的系统输入。
            if unsafe { GetLastInputInfo(&mut input) }.as_bool() {
                let changed = last.is_some_and(|old| old != input.dwTime);
                last = Some(input.dwTime);
                if changed
                    && app
                        .webview_windows()
                        .values()
                        .any(|w| w.is_focused().unwrap_or(false))
                {
                    crate::native_activity(&app);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    });
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn install(_: &tauri::AppHandle) {}
