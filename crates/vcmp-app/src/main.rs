// Release builds on Windows run without a console window.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::rc::Rc;

use slint::ComponentHandle;
use vcmp_library::Library;

fn main() -> Result<(), slint::PlatformError> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let window = vcmp_app::create_window()?;
    // Without a database VCMP still runs; preferences then last for this session only.
    let mut _albums = None;
    match open_library() {
        Ok(library) => {
            let library = Rc::new(library);
            match vcmp_app::library_view::connect(&window, library.clone()) {
                Ok(browser) => _albums = Some(browser),
                Err(error) => log::error!("could not read albums: {error}"),
            }
            if let Err(error) = vcmp_app::preferences::connect(&window, library) {
                log::warn!("could not load preferences: {error}");
            }
        }
        Err(error) => log::error!("could not open the library database: {error}"),
    }
    window.run()
}

/// Opens the library in the platform's data and cache directories, or in `$VCMP_HOME`
/// (holding `library.sqlite3` and `cache/`) when that is set, e.g. to try the app on
/// the database `cargo run --example scan` produced.
fn open_library() -> Result<Library, Box<dyn std::error::Error>> {
    if let Some(home) = std::env::var_os("VCMP_HOME") {
        let home = std::path::PathBuf::from(home);
        return Ok(Library::open(
            &home.join("library.sqlite3"),
            &home.join("cache"),
        )?);
    }
    let dirs = directories::ProjectDirs::from("", "", "vcmp")
        .ok_or("no home directory to keep VCMP's data in")?;
    Ok(Library::open(
        &dirs.data_dir().join("library.sqlite3"),
        dirs.cache_dir(),
    )?)
}
