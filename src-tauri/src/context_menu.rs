//! macOS 的右键不会像左键一样可靠地切换 first responder。
//! 多个 WKWebView 共用一个窗口时，原生菜单的编辑动作可能仍发给侧栏或旧页签。
//! 在 AppKit 打开菜单之前，把 responder 对齐到鼠标命中的网页，保留 WebKit 原生菜单。

#[cfg(target_os = "macos")]
pub fn install() {
    use block2::RcBlock;
    use objc2::{runtime::AnyClass, MainThreadMarker};
    use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags, NSEventType, NSView};
    use objc2_foundation::NSObjectProtocol;
    use std::ptr::NonNull;

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(webview_class) = AnyClass::get(c"WKWebView") else {
        return;
    };
    let handler = RcBlock::new(move |event: NonNull<NSEvent>| {
        // SAFETY: AppKit 的本地事件监听器在主线程同步调用，事件在回调期间有效。
        let event_ref = unsafe { event.as_ref() };
        let context_click = event_ref.r#type() == NSEventType::RightMouseDown
            || (event_ref.r#type() == NSEventType::LeftMouseDown
                && event_ref
                    .modifierFlags()
                    .contains(NSEventModifierFlags::Control));
        if context_click {
            if let Some(window) = event_ref.window(mtm) {
                if let Some(content) = window.contentView() {
                    // hitTest: 要父视图坐标，不能把网页 CSS 坐标或窗口坐标直接传进去。
                    // SAFETY: 主线程上读取 contentView 的父视图。
                    let parent = unsafe { content.superview() };
                    let point = parent
                        .as_ref()
                        .map_or(event_ref.locationInWindow(), |parent| {
                            parent.convertPoint_fromView(event_ref.locationInWindow(), None)
                        });
                    let mut hit = content.hitTest(point);
                    while let Some(view) = hit {
                        if view.isKindOfClass(webview_class) {
                            let already_focused =
                                window.firstResponder().is_some_and(|responder| {
                                    responder
                                        .downcast_ref::<NSView>()
                                        .is_some_and(|focused| focused.isDescendantOf(&view))
                                });
                            if !already_focused {
                                window.makeFirstResponder(Some(&view));
                            }
                            break;
                        }
                        // SAFETY: 主线程上读取命中视图的父视图，Retained 保证遍历期间存活。
                        hit = unsafe { view.superview() };
                    }
                }
            }
        }
        // 不吞鼠标事件：链接、图片、拼写、系统服务等仍由 WebKit 处理。
        event.as_ptr()
    });
    // SAFETY: 回调原样返回输入事件；监听器与应用同寿命，只在 setup 中安装一次。
    if let Some(monitor) = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(
            NSEventMask::RightMouseDown | NSEventMask::LeftMouseDown,
            &handler,
        )
    } {
        std::mem::forget(monitor);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn install() {}
