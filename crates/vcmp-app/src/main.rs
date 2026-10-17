// Release builds on Windows run without a console window.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use slint::ComponentHandle;

fn main() -> Result<(), slint::PlatformError> {
    vcmp_app::create_window()?.run()
}
