// Release builds on Windows run without a console window.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::rc::Rc;

use slint::ComponentHandle;
use vcmp_app::cover_fetch::CoverFetch;
use vcmp_app::library_service::{Delivery, LibraryPaths, LibraryService};
use vcmp_app::library_view::LibraryView;
use vcmp_app::player_service::{PlayerService, Presence, SETTING_OUTPUT_DEVICE};
use vcmp_app::tag_editor::TagEditor;
use vcmp_app::updates;
use vcmp_integrations::discord::{self, DiscordPresence};
use vcmp_integrations::lastfm::Credentials;
use vcmp_integrations::musicbrainz::{CoverFinder, CoverPicker};
use vcmp_library::Library;
use vcmp_playback::{Player, PlayerOptions};

fn main() -> Result<(), slint::PlatformError> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let window = vcmp_app::create_window()?;
    vcmp_app::shortcuts::install(&window);
    // Without a database VCMP still runs; preferences then last for this session only.
    // The services stay alive until the window closes.
    let mut services = None;
    match library_paths()
        .and_then(|paths| Ok((Library::open(&paths.database, &paths.cache)?, paths)))
    {
        Ok((library, paths)) => {
            let library = Rc::new(library);
            match LibraryView::connect(&window, library.clone()) {
                Ok(view) => {
                    // Only builds made with VCMP's Last.fm API key have Last.fm.
                    let lastfm = vcmp_app::lastfm_service::start(
                        &window,
                        library.clone(),
                        &paths,
                        Credentials::built_in(),
                    );
                    let tag_editor = TagEditor::connect(
                        &window,
                        library.clone(),
                        paths.clone(),
                        view.clone(),
                        Delivery::EventLoop,
                    );
                    let library_service = LibraryService::connect(
                        &window,
                        library.clone(),
                        paths,
                        view.clone(),
                        Delivery::EventLoop,
                    );
                    // The picker's worker asks nothing until the user searches.
                    let cover_fetch = CoverFetch::connect(
                        &window,
                        library.clone(),
                        view.clone(),
                        Box::new(CoverPicker::start(|search, event| {
                            CoverFetch::post(move |fetch| fetch.handle(search, event));
                        })),
                    );
                    let options = PlayerOptions {
                        output_device: library
                            .setting(SETTING_OUTPUT_DEVICE)
                            .ok()
                            .flatten()
                            .filter(|device| !device.is_empty()),
                        ..PlayerOptions::default()
                    };
                    let player_service = match Player::new(options) {
                        Ok((player, events)) => {
                            let presence = Presence {
                                sink: Box::new(DiscordPresence::start(
                                    discord::APPLICATION_ID,
                                    |connected| {
                                        PlayerService::post(move |service| {
                                            service.set_discord_connected(connected)
                                        });
                                    },
                                )),
                                covers: Box::new(CoverFinder::start(|album, lookup| {
                                    PlayerService::post(move |service| {
                                        service.cover_found(album, lookup)
                                    });
                                })),
                            };
                            let service = PlayerService::connect(
                                &window,
                                library.clone(),
                                view,
                                Box::new(player),
                                Some(presence),
                            );
                            PlayerService::listen(events);
                            vcmp_app::media_keys::start(&window, &service);
                            if let Some(lastfm) = &lastfm {
                                service.set_scrobbler(Box::new(lastfm.clone()));
                            }
                            let player = Rc::downgrade(&service);
                            cover_fetch.on_cover_changed(move |album| {
                                if let Some(player) = player.upgrade() {
                                    player.artwork_changed(album);
                                }
                            });
                            let player = Rc::downgrade(&service);
                            tag_editor.on_written(move || {
                                if let Some(player) = player.upgrade() {
                                    player.tags_changed();
                                }
                            });
                            Some(service)
                        }
                        Err(error) => {
                            log::error!("could not start the playback engine: {error}");
                            None
                        }
                    };
                    services = Some((
                        library_service,
                        player_service,
                        cover_fetch,
                        lastfm,
                        tag_editor,
                    ));
                }
                Err(error) => {
                    log::error!("could not read albums: {error}");
                    window.set_library_failed(true);
                }
            }
            updates::connect(&window, Some(library.clone()), updates::check_online);
            if let Err(error) = vcmp_app::preferences::connect(&window, library) {
                log::warn!("could not load preferences: {error}");
            }
        }
        Err(error) => {
            log::error!("could not open the library database: {error}");
            window.set_library_failed(true);
            updates::connect(&window, None, updates::check_online);
        }
    }
    let result = window.run();
    if let Some((_, Some(player_service), _, _, _)) = &services {
        player_service.save_queue();
    }
    result
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
