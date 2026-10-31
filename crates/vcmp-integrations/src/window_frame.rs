//! The window frame on Windows: no system title bar, so the app draws its own
//! minimise, maximise and close buttons, and the frame's border and dark mode follow
//! the app's theme.
//!
//! The window keeps every window style Windows needs for its own behaviour (snapping,
//! the window menu on Alt+Space, minimise and maximise animations); only the title
//! bar's part of the frame is given to the client area (`WM_NCCALCSIZE`). The side
//! and bottom borders stay Windows' own, invisible on Windows 10 and 11, so resizing
//! there works as with any window. The rest is hit-testing (`WM_NCHITTEST`):
//!
//! - a band at the top edge resizes, where the title bar's border was;
//! - the app's maximise button answers as Windows' own (`HTMAXBUTTON`), so Windows 11
//!   shows Snap layouts over it. Windows then sends that button's mouse messages to the
//!   frame instead of the app, so they are handled here and its hover and press are
//!   reported to the app to draw;
//! - the app's drag areas answer as the title bar (`HTCAPTION`): Windows drags the
//!   window by them, maximises it on a double-click, and snaps it at screen edges.
//!
//! The border's colour (Windows 11) and the dark frame are set through DWM. winit sets
//! the system's dark or light frame again on every `WM_SETTINGCHANGE`, so they are set
//! again after it. Elsewhere every function here does nothing: macOS and the Linux
//! desktops keep their own title bars.

/// A colour as red, green and blue.
pub type Rgb = [u8; 3];

/// Where the app's caption buttons and drag areas are, in the app's logical pixels,
/// and how many physical pixels one of those is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Caption {
    pub scale: f32,
    /// One button; the three sit side by side in the top right corner: minimise,
    /// maximise, close.
    pub button_width: f32,
    pub button_height: f32,
    /// The strip along the top of the window that drags it.
    pub drag_height: f32,
    /// A taller drag area at the top left: the sidebar, down to its first control.
    pub sidebar_width: f32,
    pub sidebar_drag_height: f32,
}

/// How the window frame looks and where its parts are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// Windows' dark frame, for the window menu and the border.
    pub dark: bool,
    pub border: Rgb,
    pub caption: Caption,
}

/// The state of the maximise button, which Windows handles for Snap layouts, for the
/// app to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaximizeButton {
    Normal,
    Hovered,
    Pressed,
}

/// What the app's caption buttons ask of the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Minimize,
    ToggleMaximized,
    Close,
}

/// Takes the title bar off the window `hwnd` (Windows' `HWND`, as a number) until it
/// closes, with `frame` for its look and hit-testing. `report` hears of the maximise
/// button's state. Once is enough; `set` gives later changes. Call it on the thread
/// that created the window. False where the window keeps the system's title bar
/// (always, off Windows), so the app shows no buttons of its own.
pub fn install(hwnd: u64, frame: Frame, report: impl Fn(MaximizeButton) + 'static) -> bool {
    platform::install(hwnd, frame, Box::new(report))
}

/// Gives an installed frame a new look or layout: the theme, the zoom or the
/// sidebar's drag area changed.
pub fn set(hwnd: u64, frame: Frame) {
    platform::set(hwnd, frame);
}

/// Does what Windows' own caption button would, through the window menu's commands,
/// so the window minimises, maximises or closes as from its title bar.
pub fn command(hwnd: u64, command: Command) {
    platform::command(hwnd, command);
}

#[cfg(windows)]
mod platform {
    use std::cell::Cell;
    use std::ffi::c_void;
    use std::ptr::null_mut;

    use windows_sys::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_BORDER_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute,
    };
    use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
    use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent,
    };
    use windows_sys::Win32::UI::Shell::{
        DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, GetClientRect, GetWindowRect, HTCAPTION, HTCLIENT, HTMAXBUTTON, HTTOP,
        HTTOPLEFT, HTTOPRIGHT, IsZoomed, NCCALCSIZE_PARAMS, PostMessageW, SC_CLOSE, SC_MAXIMIZE,
        SC_MINIMIZE, SC_RESTORE, SM_CXPADDEDBORDER, SM_CYSIZEFRAME, SWP_FRAMECHANGED,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, WM_NCCALCSIZE,
        WM_NCDESTROY, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP,
        WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_SETTINGCHANGE, WM_SYSCOMMAND, WM_THEMECHANGED,
    };
    use windows_sys::core::BOOL;

    use super::{Command, Frame, MaximizeButton, Rgb};

    /// The dark frame's attribute before Windows 10 20H1, which took 19.
    const DWMWA_USE_IMMERSIVE_DARK_MODE_BEFORE_20H1: u32 = 19;
    /// Tells this subclass apart from others on the same window.
    const SUBCLASS_ID: usize = 0x5643_4d50;

    /// What the subclass keeps for its window, behind its reference data.
    struct State {
        frame: Cell<Frame>,
        maximize: Cell<MaximizeButton>,
        report: Box<dyn Fn(MaximizeButton)>,
    }

    pub fn install(hwnd: u64, frame: Frame, report: Box<dyn Fn(MaximizeButton)>) -> bool {
        let window = hwnd as HWND;
        if state(window).is_some() {
            set(hwnd, frame);
            return true;
        }
        let state = Box::into_raw(Box::new(State {
            frame: Cell::new(frame),
            maximize: Cell::new(MaximizeButton::Normal),
            report,
        }));
        // SAFETY: the window belongs to this thread, as subclassing requires. The
        // reference data is a `State` leaked here and freed on `WM_NCDESTROY`, the
        // window's last message, so it is alive while the subclass is installed.
        unsafe {
            if SetWindowSubclass(window, Some(subclass_proc), SUBCLASS_ID, state as usize) == 0 {
                drop(Box::from_raw(state));
                log::warn!("could not subclass the window; keeping the system's title bar");
                return false;
            }
            apply_colours(window, frame);
            // The frame was measured with its title bar when the window was made;
            // measured again, through the subclass, without it.
            SetWindowPos(
                window,
                null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        refresh_size(window);
        true
    }

    pub fn set(hwnd: u64, frame: Frame) {
        let window = hwnd as HWND;
        if let Some(state) = state(window) {
            state.frame.set(frame);
            apply_colours(window, frame);
        }
    }

    /// Resizes the window by a pixel and back. Slint takes the taller client area that
    /// the frame change reports, but the OpenGL surface keeps the height it was made
    /// with until the window really changes size: without this the UI is drawn for
    /// the new height into the old one, pushed up past the window's top, until the
    /// window is resized by hand.
    fn refresh_size(window: HWND) {
        if is_maximized(window) {
            return;
        }
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        // SAFETY: reads the window's rectangle, then sets its size twice.
        unsafe {
            if GetWindowRect(window, &mut rect) == 0 {
                return;
            }
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;
            for height in [height + 1, height] {
                SetWindowPos(
                    window,
                    null_mut(),
                    0,
                    0,
                    width,
                    height,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
    }

    pub fn command(hwnd: u64, command: Command) {
        let window = hwnd as HWND;
        let command = match command {
            Command::Minimize => SC_MINIMIZE,
            Command::ToggleMaximized if is_maximized(window) => SC_RESTORE,
            Command::ToggleMaximized => SC_MAXIMIZE,
            Command::Close => SC_CLOSE,
        };
        // SAFETY: posts a window menu command, as clicking Windows' own button does.
        unsafe { PostMessageW(window, WM_SYSCOMMAND, command as WPARAM, 0) };
    }

    /// The subclass's state, if it is installed on `window`.
    fn state<'a>(window: HWND) -> Option<&'a State> {
        let mut data = 0usize;
        // SAFETY: see `install`: the data lives as long as the subclass.
        unsafe {
            (GetWindowSubclass(window, Some(subclass_proc), SUBCLASS_ID, &mut data) != 0)
                .then(|| &*(data as *const State))
        }
    }

    /// Sets the dark frame and the border's colour. Windows 10 refuses the colour, and
    /// before 20H1 knew the dark frame by another number.
    fn apply_colours(window: HWND, frame: Frame) {
        let dark = BOOL::from(frame.dark);
        if set_attribute(window, DWMWA_USE_IMMERSIVE_DARK_MODE as u32, &dark) < 0 {
            set_attribute(window, DWMWA_USE_IMMERSIVE_DARK_MODE_BEFORE_20H1, &dark);
        }
        set_attribute(window, DWMWA_BORDER_COLOR as u32, &colorref(frame.border));
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

    /// How tall Windows' resizing border is, in physical pixels, at the window's DPI.
    fn border_height(window: HWND) -> i32 {
        // SAFETY: plain queries.
        unsafe {
            let dpi = GetDpiForWindow(window);
            GetSystemMetricsForDpi(SM_CYSIZEFRAME, dpi)
                + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
        }
    }

    fn is_maximized(window: HWND) -> bool {
        // SAFETY: only reads the window's state.
        unsafe { IsZoomed(window) != 0 }
    }

    /// The client area without the title bar: Windows' own measure of the frame, with
    /// the top given back. A maximised window reaches past the screen's edge by its
    /// border, so its top keeps that much.
    fn calculate_frame(window: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        // SAFETY: with `wparam` set, `lparam` points to NCCALCSIZE_PARAMS, whose first
        // rectangle is the window's and becomes the client area's.
        unsafe {
            let params = &mut *(lparam as *mut NCCALCSIZE_PARAMS);
            let top = params.rgrc[0].top;
            let result = DefWindowProcW(window, WM_NCCALCSIZE, wparam, lparam);
            if result != 0 {
                return result;
            }
            params.rgrc[0].top = top;
            if is_maximized(window) {
                params.rgrc[0].top += border_height(window);
            }
        }
        0
    }

    /// What is under a point that Windows found in the client area.
    fn hit_test(window: HWND, state: &State, lparam: LPARAM) -> u32 {
        let mut point = POINT {
            x: i32::from(lparam as u16 as i16),
            y: i32::from((lparam >> 16) as u16 as i16),
        };
        let mut client = RECT::default();
        // SAFETY: plain queries on this thread's window.
        unsafe {
            ScreenToClient(window, &mut point);
            GetClientRect(window, &mut client);
        }
        let border = border_height(window);
        if !is_maximized(window) && point.y < border {
            return if point.x < border {
                HTTOPLEFT
            } else if point.x >= client.right - border {
                HTTOPRIGHT
            } else {
                HTTOP
            };
        }
        let caption = state.frame.get().caption;
        let x = point.x as f32 / caption.scale;
        let y = point.y as f32 / caption.scale;
        let width = client.right as f32 / caption.scale;
        if y < caption.button_height && x >= width - 3.0 * caption.button_width {
            let maximize = width - 2.0 * caption.button_width..width - caption.button_width;
            return if maximize.contains(&x) {
                HTMAXBUTTON
            } else {
                HTCLIENT
            };
        }
        if y < caption.drag_height || (x < caption.sidebar_width && y < caption.sidebar_drag_height)
        {
            return HTCAPTION;
        }
        HTCLIENT
    }

    fn report(state: &State, maximize: MaximizeButton) {
        if state.maximize.replace(maximize) != maximize {
            (state.report)(maximize);
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
        let state = data as *mut State;
        if message == WM_NCDESTROY {
            // SAFETY: the window's last message; the subclass goes before its data.
            unsafe {
                RemoveWindowSubclass(window, Some(subclass_proc), SUBCLASS_ID);
                drop(Box::from_raw(state));
                return DefSubclassProc(window, message, wparam, lparam);
            }
        }
        // SAFETY: `install` added the subclass with a live `State`, freed only above.
        let state = unsafe { &*state };
        let on_maximize = wparam == HTMAXBUTTON as WPARAM;
        match message {
            WM_NCCALCSIZE if wparam != 0 => return calculate_frame(window, wparam, lparam),
            WM_NCHITTEST => {
                // SAFETY: passes the message on unchanged. Windows answers for the side
                // and bottom borders; the client area is the app's to divide.
                let hit = unsafe { DefSubclassProc(window, message, wparam, lparam) };
                if hit != HTCLIENT as LRESULT {
                    return hit;
                }
                return hit_test(window, state, lparam) as LRESULT;
            }
            // Windows would draw its old maximise button for these, so they stop here.
            WM_NCMOUSEMOVE if on_maximize => {
                if state.maximize.get() == MaximizeButton::Normal {
                    report(state, MaximizeButton::Hovered);
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE | TME_NONCLIENT,
                        hwndTrack: window,
                        dwHoverTime: 0,
                    };
                    // SAFETY: asks for WM_NCMOUSELEAVE on this thread's window.
                    unsafe { TrackMouseEvent(&mut track) };
                }
                return 0;
            }
            WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if on_maximize => {
                report(state, MaximizeButton::Pressed);
                return 0;
            }
            WM_NCLBUTTONUP if on_maximize => {
                if state.maximize.get() == MaximizeButton::Pressed {
                    command(window as u64, Command::ToggleMaximized);
                }
                report(state, MaximizeButton::Hovered);
                return 0;
            }
            WM_NCMOUSEMOVE | WM_NCMOUSELEAVE => report(state, MaximizeButton::Normal),
            _ => {}
        }
        // SAFETY: passes the message on unchanged to winit's window procedure.
        let result = unsafe { DefSubclassProc(window, message, wparam, lparam) };
        if message == WM_SETTINGCHANGE || message == WM_THEMECHANGED {
            apply_colours(window, state.frame.get());
        }
        result
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{Command, Frame, MaximizeButton};

    pub fn install(_hwnd: u64, _frame: Frame, _report: Box<dyn Fn(MaximizeButton)>) -> bool {
        false
    }

    pub fn set(_hwnd: u64, _frame: Frame) {}

    pub fn command(_hwnd: u64, _command: Command) {}
}
