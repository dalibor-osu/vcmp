// Release builds on Windows run without a console window.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::rc::Rc;

use slint::ComponentHandle;
use vcmp_app::library_service::{Delivery, LibraryPaths, LibraryService};
use vcmp_app::library_view::LibraryView;
use vcmp_app::player_service::PlayerService;
use vcmp_library::Library;
use vcmp_playback::{Player, PlayerOptions};

fn main() -> Result<(), slint::PlatformError> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let window = vcmp_app::create_window()?;
    // Without a database VCMP still runs; preferences then last for this session only.
    // The services stay alive until the window closes.
    let mut _services = None;
    match library_paths()
        .and_then(|paths| Ok((Library::open(&paths.database, &paths.cache)?, paths)))
    {
        Ok((library, paths)) => {
            let library = Rc::new(library);
            match LibraryView::connect(&window, library.clone()) {
                Ok(view) => {
                    let library_service = LibraryService::connect(
                        &window,
                        library.clone(),
                        paths,
                        view.clone(),
                        Delivery::EventLoop,
                    );
                    let player_service = match Player::new(PlayerOptions::default()) {
                        Ok((player, events)) => {
                            let service = PlayerService::connect(
                                &window,
                                library.clone(),
                                view,
                                Box::new(player),
                            );
                            PlayerService::listen(events);
                            Some(service)
                        }
                        Err(error) => {
                            log::error!("could not start the playback engine: {error}");
                            None
                        }
                    };
                    _services = Some((library_service, player_service));
                }
                Err(error) => {
                    log::error!("could not read albums: {error}");
                    window.set_library_failed(true);
                }
            }
            if let Err(error) = vcmp_app::preferences::connect(&window, library) {
                log::warn!("could not load preferences: {error}");
            }
        }
        Err(error) => {
            log::error!("could not open the library database: {error}");
            window.set_library_failed(true);
        }
    }
    window.run()
}

/// The platform's data and cache directories, or `$VCMP_HOME` (holding
/// `library.sqlite3` and `cache/`) when that is set, e.g. to try the app on the
/// database `cargo run --example scan` produced.
fn library_paths() -> Result<LibraryPaths, Box<dyn std::error::Error>> {
    if let Some(home) = std::env::var_os("VCMP_HOME") {
        let home = PathBuf::from(home);
        return Ok(LibraryPaths {
            database: home.join("library.sqlite3"),
            cache: home.join("cache"),
        });
    }
    let dirs = directories::ProjectDirs::from("", "", "vcmp")
        .ok_or("no home directory to keep VCMP's data in")?;
    Ok(LibraryPaths {
        database: dirs.data_dir().join("library.sqlite3"),
        cache: dirs.cache_dir().to_path_buf(),
    })
}
