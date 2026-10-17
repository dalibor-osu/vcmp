//! The operating system's reduce-motion setting: Windows' "Animation effects"
//! (`SPI_GETCLIENTAREAANIMATION`), macOS' "Reduce motion" accessibility option, and
//! on Linux the desktop's setting as the XDG settings portal reports it.
//!
//! On Linux the portal's own `reduced-motion` key is asked first. Desktops whose
//! portal doesn't offer that key yet are asked for their own switch: GNOME's
//! `enable-animations`, and KDE's animation speed (`AnimationDurationFactor`, 0 for
//! "Instant").

/// Whether the operating system asks apps to reduce motion; `None` where it has no
/// such setting or it can't be read. On Linux this waits for D-Bus, up to a few
/// seconds while the portal starts, so call it off the UI thread.
pub fn system_setting() -> Option<bool> {
    let setting = platform::read();
    log::debug!("system reduce-motion setting: {setting:?}");
    setting
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
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
}

#[cfg(target_os = "macos")]
mod platform {
    use objc2_app_kit::NSWorkspace;

    pub fn read() -> Option<bool> {
        Some(NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion())
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
        let connection = connection::Builder::session()
            .and_then(|builder| builder.method_timeout(TIMEOUT).build())
            .inspect_err(|error| log::debug!("no D-Bus session for the settings portal: {error}"))
            .ok()?;
        let portal = Proxy::new(
            &connection,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Settings",
        )
        .ok()?;
        SETTINGS.into_iter().find_map(|(namespace, key)| {
            let value = read_one(&portal, namespace, key)?;
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
