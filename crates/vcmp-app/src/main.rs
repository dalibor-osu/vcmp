// Release builds on Windows run without a console window.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::rc::Rc;

use slint::ComponentHandle;
use vcmp_library::Library;

fn main() -> Result<(), slint::PlatformError> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let window = vcmp_app::create_window()?;
    // Without a database VCMP still runs; preferences then last for this session only.
    match open_library() {
        Ok(library) => {
            if let Err(error) = vcmp_app::preferences::connect(&window, Rc::new(library)) {
                log::warn!("could not load preferences: {error}");
            }
        }
        Err(error) => log::error!("could not open the library database: {error}"),
    }
    window.run()
}

fn open_library() -> Result<Library, Box<dyn std::error::Error>> {
    let dirs = directories::ProjectDirs::from("", "", "vcmp")
        .ok_or("no home directory to keep VCMP's data in")?;
    Ok(Library::open(
        &dirs.data_dir().join("library.sqlite3"),
        dirs.cache_dir(),
    )?)
}
