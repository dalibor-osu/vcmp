//! Updates in the window: the check's switches in Settings, saved in the library
//! database's settings, its note in the sidebar and in Settings' About section, and the
//! update dialog, which installs a release and hands over to it. The check is
//! `vcmp_integrations::updates`, the install and hand-over
//! `vcmp_integrations::self_update`.

use std::cell::{Cell, RefCell};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use slint::ComponentHandle;
use vcmp_integrations::self_update::{self, Blocker, Failure, HandOver, Plan};
use vcmp_integrations::updates::{self, Release};
use vcmp_library::Library;

use crate::{AppWindow, UpdateBlocker, UpdateFailure, UpdatePhase, UpdateState};

/// Setting key: check for updates when VCMP starts ("true" or "false"; absent: true).
pub const SETTING_UPDATE_CHECK: &str = "integrations.update_check";
/// Setting key: releases flagged as pre-releases count ("true" or "false"; absent:
/// false).
pub const SETTING_UPDATE_PRERELEASES: &str = "integrations.update_prereleases";

const RELEASE_PAGE: &str = "https://github.com/dalibor-osu/vcmp/releases/tag/v";

thread_local! {
    /// The install the dialog offers, for the release on show.
    static PLAN: RefCell<Option<Plan>> = const { RefCell::new(None) };
    /// Set to stop the download that runs.
    static CANCEL: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
    /// The new version in place, for `main` to start once the window has closed.
    static HAND_OVER: RefCell<Option<HandOver>> = const { RefCell::new(None) };
}

/// Asks for a newer release; given the window to show it in.
type Check = Rc<dyn Fn(slint::Weak<AppWindow>)>;

/// A check that waits, while the setup wizard is open on the first start, until
/// [`HeldCheck::release`]: nothing goes to GitHub before the user has seen the switch.
pub struct HeldCheck {
    held: Rc<Cell<bool>>,
    run: Rc<dyn Fn(&AppWindow, bool)>,
}

impl HeldCheck {
    /// Lets the check run: now if the switch is still on, or when it is turned on.
    pub fn release(&self, window: &AppWindow) {
        self.held.set(false);
        if window.global::<UpdateState>().get_check_enabled() {
            (self.run)(window, false);
        }
    }
}

/// Applies the saved switches and saves them when the user flips them. `check` runs
/// once: now if the check is on, or later when the user turns it on; and again when
/// the user changes whether pre-releases count. Without a database the switches last
/// for the session.
pub fn connect(
    window: &AppWindow,
    library: Option<Rc<Library>>,
    check: impl Fn(slint::Weak<AppWindow>) + 'static,
) {
    connect_held(window, library, false, check);
}

/// As [`connect`], but with `held` the check waits for the returned
/// [`HeldCheck::release`], even with the switch on.
pub fn connect_held(
    window: &AppWindow,
    library: Option<Rc<Library>>,
    held: bool,
    check: impl Fn(slint::Weak<AppWindow>) + 'static,
) -> HeldCheck {
    let saved = |key: &str| {
        library.as_ref().and_then(|library| {
            library
                .setting(key)
                .inspect_err(|error| log::warn!("could not read the {key} setting: {error}"))
                .ok()
                .flatten()
        })
    };
    let enabled = saved(SETTING_UPDATE_CHECK).as_deref() != Some("false");
    let prereleases = saved(SETTING_UPDATE_PRERELEASES).as_deref() == Some("true");
    let state = window.global::<UpdateState>();
    state.set_check_enabled(enabled);
    state.set_include_prereleases(prereleases);
    state.set_current_version(env!("CARGO_PKG_VERSION").into());

    let check: Check = Rc::new(check);
    let ran = Rc::new(Cell::new(false));
    let held = Rc::new(Cell::new(held));
    let run: Rc<dyn Fn(&AppWindow, bool)> = Rc::new({
        let held = held.clone();
        move |window: &AppWindow, again: bool| {
            if held.get() || (ran.get() && !again) {
                return;
            }
            ran.set(true);
            check(window.as_weak());
        }
    });
    if enabled {
        run(window, false);
    }

    let save = move |key: &str, on: bool| {
        if let Some(library) = &library
            && let Err(error) = library.set_setting(key, if on { "true" } else { "false" })
        {
            log::warn!("could not save the {key} setting: {error}");
        }
    };
    let save = Rc::new(save);
    let weak = window.as_weak();
    let switched = run.clone();
    let save_check = save.clone();
    state.on_set_check_enabled(move |on| {
        let Some(window) = weak.upgrade() else { return };
        window.global::<UpdateState>().set_check_enabled(on);
        save_check(SETTING_UPDATE_CHECK, on);
        if on {
            switched(&window, false);
        }
    });
    let weak = window.as_weak();
    let rerun = run.clone();
    state.on_set_include_prereleases(move |on| {
        let Some(window) = weak.upgrade() else { return };
        let state = window.global::<UpdateState>();
        state.set_include_prereleases(on);
        save(SETTING_UPDATE_PRERELEASES, on);
        if state.get_phase() == UpdatePhase::Offer && !state.get_dialog_open() {
            forget(&window);
        }
        if state.get_check_enabled() {
            rerun(&window, true);
        }
    });

    let weak = window.as_weak();
    state.on_install(move || {
        if let Some(window) = weak.upgrade() {
            install(&window);
        }
    });
    state.on_cancel_install(|| {
        CANCEL.with_borrow(|cancel| {
            if let Some(cancel) = cancel {
                cancel.store(true, Ordering::Relaxed);
            }
        });
    });
    HeldCheck { held, run }
}

/// Asks GitHub on a worker thread and shows a newer release in the window. The check
/// `main` passes to `connect`.
pub fn check_online(window: slint::Weak<AppWindow>) {
    let Some(prereleases) = window
        .upgrade()
        .map(|window| window.global::<UpdateState>().get_include_prereleases())
    else {
        return;
    };
    updates::start(env!("CARGO_PKG_VERSION"), prereleases, move |release| {
        let _ = window.upgrade_in_event_loop(move |window| show(&window, &release));
    });
}

/// Shows that `release` is out, and whether this copy can install it itself. The
/// note stays hidden while the check is off; a pre-release found before the user
/// turned pre-releases off is dropped, and nothing changes while an install runs.
pub fn show(window: &AppWindow, release: &Release) {
    let program = std::env::current_exe().unwrap_or_default();
    show_with(window, release, self_update::plan(release, &program));
}

/// [`show`], with the answer to whether it can be installed given.
pub fn show_with(window: &AppWindow, release: &Release, plan: Result<Plan, Blocker>) {
    let state = window.global::<UpdateState>();
    if release.prerelease && !state.get_include_prereleases() {
        return;
    }
    if state.get_dialog_open() && state.get_phase() != UpdatePhase::Offer {
        return;
    }
    state.set_release_url(release.url.as_str().into());
    state.set_version(release.version.as_str().into());
    state.set_prerelease(release.prerelease);
    state.set_blocker(match &plan {
        Ok(_) => UpdateBlocker::None,
        Err(Blocker::Platform) => UpdateBlocker::Platform,
        Err(Blocker::DebugBuild) => UpdateBlocker::DebugBuild,
        Err(Blocker::NoKey) => UpdateBlocker::NoKey,
        Err(Blocker::NotSigned) => UpdateBlocker::NotSigned,
        Err(Blocker::ReadOnly) => UpdateBlocker::ReadOnly,
    });
    PLAN.set(plan.ok());
}

/// Forgets the release on show, so that the next check's answer decides.
fn forget(window: &AppWindow) {
    let state = window.global::<UpdateState>();
    state.set_version("".into());
    state.set_release_url("".into());
    state.set_prerelease(false);
    PLAN.set(None);
}

/// Starts installing the release on show, if this copy can.
fn install(window: &AppWindow) {
    let Some(plan) = PLAN.with_borrow(Clone::clone) else {
        return;
    };
    let state = window.global::<UpdateState>();
    state.set_phase(UpdatePhase::Downloading);
    state.set_progress(0.0);
    state.set_error("".into());
    let progress = window.as_weak();
    let done = window.as_weak();
    let cancel = self_update::start_install(
        plan,
        move |fraction| {
            let _ = progress.upgrade_in_event_loop(move |window| {
                window.global::<UpdateState>().set_progress(fraction);
            });
        },
        move |result| {
            let _ = done.upgrade_in_event_loop(move |window| installed(&window, result));
        },
    );
    CANCEL.set(Some(cancel));
}

/// Shows how the install ended; with the new version in place, closes the window so
/// that `main` can hand over to it.
pub fn installed(window: &AppWindow, result: Result<HandOver, Failure>) {
    CANCEL.set(None);
    let state = window.global::<UpdateState>();
    match result {
        Ok(hand_over) => {
            state.set_phase(UpdatePhase::Restarting);
            HAND_OVER.set(Some(hand_over));
            let _ = slint::quit_event_loop();
        }
        Err(Failure::Cancelled) => state.set_phase(UpdatePhase::Offer),
        Err(failure) => {
            state.set_failure(match failure {
                Failure::Download(_) | Failure::Cancelled => UpdateFailure::Download,
                Failure::Verify(_) => UpdateFailure::Verify,
                Failure::Install(_) => UpdateFailure::Install,
            });
            state.set_error(failure.to_string().into());
            state.set_phase(UpdatePhase::Failed);
        }
    }
}

/// The new version waiting to be started, once, after the window has closed.
pub fn take_hand_over() -> Option<HandOver> {
    HAND_OVER.take()
}

/// What the program was started for, from its arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartReason {
    /// An ordinary start.
    Normal,
    /// Started by the version before, which waits for the confirmation at this path.
    Updated(PathBuf),
    /// Started again after this version failed to start.
    UpdateFailed(String),
}

impl StartReason {
    /// Reads the arguments after the program's name.
    pub fn from_args(mut args: impl Iterator<Item = OsString>) -> Self {
        let first = args.next();
        let second = args.next();
        match (first.as_ref().and_then(|arg| arg.to_str()), second) {
            (Some(self_update::STARTED_ARG), Some(marker)) => Self::Updated(marker.into()),
            (Some(self_update::FAILED_ARG), Some(version)) => {
                Self::UpdateFailed(version.to_string_lossy().into_owned())
            }
            _ => Self::Normal,
        }
    }
}

/// Tells the version before that this one is up.
pub fn confirm_started(marker: &Path) {
    self_update::confirm_started(marker);
}

/// Opens the dialog to say that `version` didn't start and this one was put back.
pub fn show_rolled_back(window: &AppWindow, version: &str) {
    let state = window.global::<UpdateState>();
    state.set_failed_version(version.into());
    state.set_release_url(format!("{RELEASE_PAGE}{version}").into());
    state.set_phase(UpdatePhase::RolledBack);
    state.set_dialog_open(true);
}
