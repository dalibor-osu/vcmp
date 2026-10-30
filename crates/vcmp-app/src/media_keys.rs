//! Registers VCMP with the OS media controls once the window is up: Windows attaches
//! them to the native window, which exists only after the event loop has created it.

use std::rc::{Rc, Weak};
use std::time::Duration;

use slint::ComponentHandle;
use vcmp_integrations::media_controls::MediaControls;

use crate::AppWindow;
use crate::player_service::PlayerService;

/// How often, and how long apart, the window handle is asked for before giving up.
const ATTEMPTS: u32 = 50;
const RETRY: Duration = Duration::from_millis(100);

/// Starts the media controls for `service` from the running event loop.
pub fn start(window: &AppWindow, service: &Rc<PlayerService>) {
    let window = window.as_weak();
    let service = Rc::downgrade(service);
    slint::Timer::single_shot(Duration::ZERO, move || attach(window, service, ATTEMPTS));
}

fn attach(window: slint::Weak<AppWindow>, service: Weak<PlayerService>, attempts: u32) {
    let (Some(app), Some(player)) = (window.upgrade(), service.upgrade()) else {
        return;
    };
    let handle = native_handle(&app);
    if cfg!(windows) && handle.is_none() {
        if attempts > 0 {
            slint::Timer::single_shot(RETRY, move || attach(window, service, attempts - 1));
        } else {
            log::warn!("no window handle for the media controls; running without them");
        }
        return;
    }
    match MediaControls::start(handle, |control| {
        PlayerService::post(move |service| service.media_control(control));
    }) {
        Ok(controls) => player.set_media_controls(Box::new(controls)),
        Err(error) => log::info!("running without OS media controls: {error}"),
    }
}

/// The window's `HWND`, which the Windows media controls attach to and the window
/// frame is changed through (`src/title_bar.rs`); `None` off Windows.
#[cfg(windows)]
pub(crate) fn native_handle(window: &AppWindow) -> Option<u64> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let handle = window.window().window_handle();
    match handle.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(win32) => Some(win32.hwnd.get() as u64),
        _ => None,
    }
}

#[cfg(not(windows))]
pub(crate) fn native_handle(_window: &AppWindow) -> Option<u64> {
    None
}
