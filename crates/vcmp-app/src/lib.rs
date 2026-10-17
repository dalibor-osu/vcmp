//! VCMP's desktop front end: the Slint UI and the glue that feeds it from the core
//! crates.

pub mod albums;
pub mod library_service;
pub mod library_view;
pub mod preferences;
pub mod theme;

slint::include_modules!();

/// Creates the main window with the theme palettes installed.
pub fn create_window() -> Result<AppWindow, slint::PlatformError> {
    let window = AppWindow::new()?;
    theme::install(&window);
    window.set_version(env!("CARGO_PKG_VERSION").into());
    Ok(window)
}
