//! The operating system's reduce-motion setting: Windows' "Animation effects"
//! (`SPI_GETCLIENTAREAANIMATION`), macOS' "Reduce motion" accessibility option, and
//! on Linux the desktop's setting as the XDG settings portal reports it.
//!
//! On Linux the portal's own `reduced-motion` key is asked first. Desktops whose
//! portal doesn't offer that key yet are asked for their own switch: GNOME's
//! `enable-animations`, and KDE's animation speed (`AnimationDurationFactor`, 0 for
//! "Instant").
//!
//! `follow` also hears about changes: the portal's `SettingChanged` signal on Linux,
//! `WM_SETTINGCHANGE` on Windows, and AppKit's accessibility display notification on
//! macOS. Each one makes it read the setting again.

use std::thread;

/// Whether the operating system asks apps to reduce motion; `None` where it has no
/// such setting or it can't be read. On Linux this waits for D-Bus, up to a few
/// seconds while the portal starts, so call it off the UI thread.
pub fn system_setting() -> Option<bool> {
    let setting = platform::read();
    log::debug!("system reduce-motion setting: {setting:?}");
    setting
}

/// Calls `report` with whether the operating system asks apps to reduce motion: once
/// it is known, then whenever it changes, for as long as VCMP runs. Nothing is
/// reported where the system has no such setting. Runs on a worker thread, except
/// that macOS reports changes on its main thread.
pub fn follow(mut report: impl FnMut(bool) + Send + 'static) {
    let mut changes = Changes::default();
    let spawned = thread::Builder::new()
        .name("vcmp-reduce-motion".into())
        .spawn(move || {
            platform::follow(move |setting| {
                if let Some(reduce) = changes.update(setting) {
                    log::debug!("system reduce-motion setting: {reduce}");
                    report(reduce);
                }
            });
        });
    if let Err(error) = spawned {
        log::warn!("could not follow the system's reduce-motion setting: {error}");
    }
}

/// Passes a setting on only when it differs from the last one passed on. A setting
/// that can no longer be read keeps the last one.
#[derive(Default)]
struct Changes {
    last: Option<bool>,
}

impl Changes {
    fn update(&mut self, setting: Option<bool>) -> Option<bool> {
        let setting = setting?;
        if self.last == Some(setting) {
            return None;
        }
        self.last = Some(setting);
        Some(setting)
    }
}

#[cfg(windows)]
mod platform {
    use std::cell::RefCell;
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, MSG, RegisterClassW,
        SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW, WM_SETTINGCHANGE, WNDCLASSW,
    };
    use windows_sys::core::BOOL;

    pub fn read() -> Option<bool> {
        let mut animate: BOOL = 1;
        // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL through the pointer.
        let read = unsafe {
            SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut animate).cast(), 0)
        };
        (read != 0).then_some(animate == 0)
    }

    thread_local! {
        /// What `window_proc` calls on this thread when a system setting changes.
        static CHANGED: RefCell<Option<Box<dyn FnMut(Option<bool>)>>> =
            const { RefCell::new(None) };
    }

    /// Reports the setting, then waits for `WM_SETTINGCHANGE` in this thread's message
    /// loop for as long as VCMP runs. The window that receives it is a hidden top-level
    /// one: Windows broadcasts the message to top-level windows only, not to
    /// message-only ones.
    pub fn follow(mut changed: impl FnMut(Option<bool>) + 'static) {
        changed(read());
        CHANGED.set(Some(Box::new(changed)));
        let class: Vec<u16> = "VcmpReduceMotion\0".encode_utf16().collect();
        // SAFETY: the class and window outlive the message loop, which never returns
        // while VCMP runs; `class` stays alive across all three calls that read it.
        unsafe {
            let instance = GetModuleHandleW(null());
            let window_class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..Default::default()
            };
            if RegisterClassW(&window_class) == 0 {
                log::debug!("could not register the reduce-motion window class");
                return;
            }
            let window = CreateWindowExW(
                0,
                class.as_ptr(),
                null(),
                0,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            if window.is_null() {
                log::debug!("could not create the reduce-motion window");
                return;
            }
            let mut message = MSG::default();
            while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
                DispatchMessageW(&message);
            }
        }
    }

    /// Any setting may have changed; reading ours again is cheap.
    unsafe extern "system" fn window_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == WM_SETTINGCHANGE {
            CHANGED.with_borrow_mut(|changed| {
                if let Some(changed) = changed {
                    changed(read());
                }
            });
        }
        // SAFETY: passes the message on unchanged, as every window procedure must.
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ptr::NonNull;
    use std::sync::Mutex;

    use block2::RcBlock;
    use objc2_app_kit::{NSWorkspace, NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification};
    use objc2_foundation::NSNotification;

    pub fn read() -> Option<bool> {
        Some(NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion())
    }

    /// Reports the setting, then asks AppKit to call back when the accessibility
    /// display options change. AppKit posts that on the main thread, from the run
    /// loop the window's event loop runs.
    pub fn follow(mut changed: impl FnMut(Option<bool>) + Send + 'static) {
        changed(read());
        let changed = Mutex::new(changed);
        let block = RcBlock::new(move |_: NonNull<NSNotification>| {
            if let Ok(mut changed) = changed.lock() {
                changed(read());
            }
        });
        let workspace = NSWorkspace::sharedWorkspace();
        // SAFETY: the name is AppKit's own constant, no object or queue is given, and
        // the block is sendable: its only state is behind a Mutex.
        let observer = unsafe {
            workspace
                .notificationCenter()
                .addObserverForName_object_queue_usingBlock(
                    Some(NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification),
                    None,
                    None,
                    &block,
                )
        };
        // Observed for as long as VCMP runs, so the observer is never removed.
        std::mem::forget(observer);
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::time::Duration;

    use zbus::blocking::{Proxy, connection};
    use zbus::zvariant::{OwnedValue, Value};

    /// Asked in this order; the first setting the portal has decides.
    const SETTINGS: [(&str, &str); 3] = [
        ("org.freedesktop.appearance", "reduced-motion"),
        ("org.gnome.desktop.interface", "enable-animations"),
        ("org.kde.kdeglobals.KDE", "AnimationDurationFactor"),
    ];

    /// Long enough for D-Bus to start the portal on the first question.
    const TIMEOUT: Duration = Duration::from_secs(5);

    pub fn read() -> Option<bool> {
        read_from(&portal()?)
    }

    /// Reports the setting, then reads it again after each `SettingChanged` signal
    /// for one of `SETTINGS`, for as long as VCMP runs.
    pub fn follow(mut changed: impl FnMut(Option<bool>)) {
        let Some(portal) = portal() else { return };
        // Subscribed before the first read, so a change between the two isn't missed.
        let signals = portal
            .receive_signal("SettingChanged")
            .inspect_err(|error| log::debug!("can't follow the settings portal: {error}"));
        changed(read_from(&portal));
        let Ok(signals) = signals else { return };
        for signal in signals {
            let Ok((namespace, key, _)) =
                signal.body().deserialize::<(String, String, OwnedValue)>()
            else {
                continue;
            };
            if SETTINGS.contains(&(namespace.as_str(), key.as_str())) {
                changed(read_from(&portal));
            }
        }
    }

    fn portal() -> Option<Proxy<'static>> {
        let connection = connection::Builder::session()
            .and_then(|builder| builder.method_timeout(TIMEOUT).build())
            .inspect_err(|error| log::debug!("no D-Bus session for the settings portal: {error}"))
            .ok()?;
        Proxy::new(
            &connection,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Settings",
        )
        .ok()
    }

    fn read_from(portal: &Proxy) -> Option<bool> {
        SETTINGS.into_iter().find_map(|(namespace, key)| {
            let value = read_one(portal, namespace, key)?;
            reduces_motion(key, &value)
        })
    }

    /// One setting's value, or `None` when the portal doesn't have it.
    fn read_one(portal: &Proxy, namespace: &str, key: &str) -> Option<OwnedValue> {
        match portal.call("ReadOne", &(namespace, key)) {
            Ok(value) => Some(value),
            // Portals before version 2 only have `Read`, which wraps the value in one
            // more variant; `reduces_motion` looks inside it.
            Err(zbus::Error::MethodError(name, ..))
                if name.as_str() == "org.freedesktop.DBus.Error.UnknownMethod" =>
            {
                portal.call("Read", &(namespace, key)).ok()
            }
            Err(error) => {
                log::debug!("settings portal has no {namespace} {key}: {error}");
                None
            }
        }
    }

    /// What a setting's value says: `Some(true)` asks to reduce motion. `None` for a
    /// value of a type the key never has.
    pub(super) fn reduces_motion(key: &str, value: &Value) -> Option<bool> {
        match (key, value) {
            (_, Value::Value(inner)) => reduces_motion(key, inner),
            // 1 is "reduced motion"; 0 and unknown values are "no preference".
            ("reduced-motion", Value::U32(preference)) => Some(*preference == 1),
            ("enable-animations", Value::Bool(enabled)) => Some(!enabled),
            // KDE's portal passes kdeglobals entries on as text.
            ("AnimationDurationFactor", Value::Str(factor)) => {
                factor.parse::<f64>().ok().map(|factor| factor <= 0.0)
            }
            ("AnimationDurationFactor", Value::F64(factor)) => Some(*factor <= 0.0),
            _ => None,
        }
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
mod platform {
    pub fn read() -> Option<bool> {
        None
    }

    pub fn follow(_changed: impl FnMut(Option<bool>)) {}
}

#[cfg(test)]
mod change_tests {
    use super::Changes;

    #[test]
    fn only_changes_are_passed_on() {
        let mut changes = Changes::default();
        assert_eq!(changes.update(None), None, "nothing known yet");
        assert_eq!(changes.update(Some(false)), Some(false), "the first answer");
        assert_eq!(changes.update(Some(false)), None, "the same answer again");
        assert_eq!(changes.update(Some(true)), Some(true));
        assert_eq!(
            changes.update(None),
            None,
            "an unreadable setting keeps the last one"
        );
        assert_eq!(changes.update(Some(true)), None);
        assert_eq!(changes.update(Some(false)), Some(false));
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use zbus::zvariant::Value;

    use super::platform::reduces_motion;

    #[test]
    fn portal_key_asks_for_reduced_motion_with_1() {
        assert_eq!(reduces_motion("reduced-motion", &Value::U32(1)), Some(true));
        assert_eq!(
            reduces_motion("reduced-motion", &Value::U32(0)),
            Some(false)
        );
        assert_eq!(
            reduces_motion("reduced-motion", &Value::U32(7)),
            Some(false),
            "unknown values mean no preference"
        );
    }

    #[test]
    fn gnome_turns_animations_off() {
        assert_eq!(
            reduces_motion("enable-animations", &Value::Bool(false)),
            Some(true)
        );
        assert_eq!(
            reduces_motion("enable-animations", &Value::Bool(true)),
            Some(false)
        );
    }

    #[test]
    fn kde_instant_animations_reduce_motion() {
        let factor = "AnimationDurationFactor";
        assert_eq!(reduces_motion(factor, &Value::from("0")), Some(true));
        assert_eq!(reduces_motion(factor, &Value::from("0.5")), Some(false));
        assert_eq!(reduces_motion(factor, &Value::F64(0.0)), Some(true));
        assert_eq!(reduces_motion(factor, &Value::from("fast")), None);
    }

    #[test]
    fn values_wrapped_by_the_old_read_method_are_unwrapped() {
        let wrapped = Value::Value(Box::new(Value::Bool(false)));
        assert_eq!(reduces_motion("enable-animations", &wrapped), Some(true));
    }

    #[test]
    fn values_of_the_wrong_type_say_nothing() {
        assert_eq!(reduces_motion("enable-animations", &Value::U32(0)), None);
        assert_eq!(reduces_motion("reduced-motion", &Value::Bool(true)), None);
    }
}
