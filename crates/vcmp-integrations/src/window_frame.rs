//! The window frame's colours on Windows: the title bar, its text and the border take
//! colours the app gives instead of the system's, which are often its accent colour
//! ("Show accent colour on title bars and window borders").
//!
//! They are set through DWM. Windows 11 takes all three colours; Windows 10 (from
//! 1809) only a dark or light frame, and keeps the accent colour on the title bar when
//! that setting is on. winit sets the system's dark or light frame again on every
//! `WM_SETTINGCHANGE`, so the window is subclassed to set the app's colours again after
//! winit has handled it. The title is drawn in the inactive text colour while the
//! window isn't active, as Windows dims its own. Elsewhere `set` does nothing: macOS
//! and the Linux desktops draw their own frames, in the system's light or dark mode.

/// A colour as red, green and blue.
pub type Rgb = [u8; 3];

/// The colours a window frame takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameColours {
    /// Windows' dark frame: light caption buttons on a dark title bar.
    pub dark: bool,
    pub caption: Rgb,
    pub text: Rgb,
    pub inactive_text: Rgb,
    pub border: Rgb,
}

/// Gives the window `hwnd` (Windows' `HWND`, as a number) these colours, and keeps
/// them through system setting changes until the window closes. Call it again with new
/// colours whenever they change, on the thread that created the window.
pub fn set(hwnd: u64, colours: FrameColours) {
    platform::set(hwnd, colours);
}

#[cfg(windows)]
mod platform {
    use std::cell::Cell;
    use std::ffi::c_void;

    use windows_sys::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
        DwmSetWindowAttribute,
    };
    use windows_sys::Win32::UI::Shell::{
        DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, GetForegroundWindow, WM_NCACTIVATE, WM_NCDESTROY, WM_SETTINGCHANGE,
        WM_THEMECHANGED,
    };
    use windows_sys::core::BOOL;

    use super::{FrameColours, Rgb};

    /// The dark frame's attribute before Windows 10 20H1, which took 19.
    const DWMWA_USE_IMMERSIVE_DARK_MODE_BEFORE_20H1: u32 = 19;
    /// Tells this subclass apart from others on the same window.
    const SUBCLASS_ID: usize = 0x5643_4d50;

    /// What the subclass keeps for its window, behind its reference data.
    struct Frame {
        colours: Cell<FrameColours>,
        active: Cell<bool>,
    }

    pub fn set(hwnd: u64, colours: FrameColours) {
        let window = hwnd as HWND;
        let mut data = 0usize;
        // SAFETY: the window belongs to this thread, as subclassing requires. The
        // reference data is a `Frame` leaked here and freed on `WM_NCDESTROY`, the
        // window's last message, so it is alive while the subclass is installed.
        unsafe {
            if GetWindowSubclass(window, Some(subclass_proc), SUBCLASS_ID, &mut data) != 0 {
                let frame = &*(data as *const Frame);
                let dark_changed = frame.colours.get().dark != colours.dark;
                frame.colours.set(colours);
                apply(window, frame);
                if dark_changed {
                    repaint_caption(window, frame.active.get());
                }
                return;
            }
            let frame = Box::into_raw(Box::new(Frame {
                colours: Cell::new(colours),
                active: Cell::new(GetForegroundWindow() == window),
            }));
            if SetWindowSubclass(window, Some(subclass_proc), SUBCLASS_ID, frame as usize) == 0 {
                drop(Box::from_raw(frame));
                log::debug!("could not subclass the window to keep its frame colours");
                return;
            }
            apply(window, &*frame);
            repaint_caption(window, (*frame).active.get());
        }
    }

    /// Sets the frame's colours. Windows 10 refuses the three colours, and before 20H1
    /// knew the dark frame by another number.
    fn apply(window: HWND, frame: &Frame) {
        let colours = frame.colours.get();
        let dark = BOOL::from(colours.dark);
        let text = if frame.active.get() {
            colours.text
        } else {
            colours.inactive_text
        };
        if set_attribute(window, DWMWA_USE_IMMERSIVE_DARK_MODE as u32, &dark) < 0 {
            set_attribute(window, DWMWA_USE_IMMERSIVE_DARK_MODE_BEFORE_20H1, &dark);
        }
        set_attribute(
            window,
            DWMWA_CAPTION_COLOR as u32,
            &colorref(colours.caption),
        );
        set_attribute(window, DWMWA_TEXT_COLOR as u32, &colorref(text));
        set_attribute(window, DWMWA_BORDER_COLOR as u32, &colorref(colours.border));
    }

    fn set_attribute<T>(window: HWND, attribute: u32, value: &T) -> i32 {
        // SAFETY: DWM reads `size_of::<T>()` bytes from `value`, which is a BOOL or a
        // COLORREF, the types these attributes take.
        unsafe {
            DwmSetWindowAttribute(
                window,
                attribute,
                (value as *const T).cast::<c_void>(),
                size_of::<T>() as u32,
            )
        }
    }

    /// `0x00bbggrr`.
    fn colorref([red, green, blue]: Rgb) -> COLORREF {
        u32::from(red) | u32::from(green) << 8 | u32::from(blue) << 16
    }

    /// Windows 10 draws a changed dark frame only when the window's activation next
    /// changes, so the caption is drawn inactive and back. `DefWindowProcW` is called
    /// directly, so the app never hears of a focus change.
    fn repaint_caption(window: HWND, active: bool) {
        // SAFETY: WM_NCACTIVATE with no update region only redraws the non-client area.
        unsafe {
            DefWindowProcW(window, WM_NCACTIVATE, WPARAM::from(!active), 0);
            DefWindowProcW(window, WM_NCACTIVATE, WPARAM::from(active), 0);
        }
    }

    unsafe extern "system" fn subclass_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        data: usize,
    ) -> LRESULT {
        let frame = data as *mut Frame;
        if message == WM_NCDESTROY {
            // SAFETY: the window's last message; the subclass goes before its data.
            unsafe {
                RemoveWindowSubclass(window, Some(subclass_proc), SUBCLASS_ID);
                drop(Box::from_raw(frame));
                return DefSubclassProc(window, message, wparam, lparam);
            }
        }
        // SAFETY: `set` installed the subclass with a live `Frame`, freed only above.
        let frame = unsafe { &*frame };
        if message == WM_NCACTIVATE {
            // The title's colour first, so the caption is drawn in it.
            frame.active.set(wparam != 0);
            apply(window, frame);
        }
        // SAFETY: passes the message on unchanged to winit's window procedure.
        let result = unsafe { DefSubclassProc(window, message, wparam, lparam) };
        if message == WM_SETTINGCHANGE || message == WM_THEMECHANGED {
            apply(window, frame);
        }
        result
    }
}

#[cfg(not(windows))]
mod platform {
    use super::FrameColours;

    pub fn set(_hwnd: u64, _colours: FrameColours) {}
}
