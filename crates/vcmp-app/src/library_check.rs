//! The Library check page behind the `LibraryCheck` global: duplicate tracks, tracks
//! with missing tags and albums without a cover, as `vcmp-library` finds them. The
//! findings are read when the page opens and again after every library change while
//! it is open (`LibraryView::reload`), so an edit or a found cover takes its entry
//! away.
//!
//! The page acts only through the paths the rest of VCMP already has: the tag editor
//! (`TagEditorState`), the cover picker (`CoverSearch`), the player (`on_play`) and
//! the file manager. It never writes, moves or removes a music file itself; the one
//! thing it stores is "Not duplicates".

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use vcmp_integrations::file_manager;
use vcmp_library::{Album, AlbumGaps, DuplicateGroup, Library, Track, TrackGaps};

use crate::library_view::format_duration;
use crate::{
    AppWindow, CheckList, CheckTab, CheckTrack, CoverSearch, CoverlessAlbum, DuplicateBlock,
    GapBlock, LibraryCheck, TagEditorState,
};

/// How many groups, albums or covers a tab adds at a time ("Show more").
pub const PAGE_SIZE: usize = 100;

/// The longest path a row shows before its middle gives way to an ellipsis.
const PATH_CHARS: usize = 90;

/// Told to play tracks: the list, the queue panel's context (none, or an album) and
/// the track to start from.
type Play = Box<dyn Fn(&[Track], Option<String>, usize)>;

pub struct CheckPage {
    window: slint::Weak<AppWindow>,
    library: Rc<Library>,
    // Every duplicate group, hidden ones included, and those the page lists now.
    all_groups: RefCell<Vec<DuplicateGroup>>,
    groups: RefCell<Vec<DuplicateGroup>>,
    gaps: RefCell<Vec<AlbumGaps>>,
    coverless: RefCell<Vec<Album>>,
    // How many entries each tab lists, in `CheckTab` order.
    shown: Cell<[usize; 3]>,
    on_play: RefCell<Option<Play>>,
}

impl CheckPage {
    /// Connects the `LibraryCheck` global. Keep the page alive as long as the window.
    pub fn connect(window: &AppWindow, library: Rc<Library>) -> Rc<Self> {
        let page = Rc::new(Self {
            window: window.as_weak(),
            library,
            all_groups: RefCell::default(),
            groups: RefCell::default(),
            gaps: RefCell::default(),
            coverless: RefCell::default(),
            shown: Cell::new([PAGE_SIZE; 3]),
            on_play: RefCell::default(),
        });
        let state = window.global::<LibraryCheck>();
        let weak = Rc::downgrade(&page);
        state.on_show(move || {
            if let Some(page) = weak.upgrade() {
                page.open();
            }
        });
        let weak = Rc::downgrade(&page);
        state.on_show_more(move |tab| {
            if let Some(page) = weak.upgrade() {
                let mut shown = page.shown.get();
                shown[tab_index(tab)] += PAGE_SIZE;
                page.shown.set(shown);
                page.show();
            }
        });
        let weak = Rc::downgrade(&page);
        state.on_set_show_hidden(move |on| {
            if let Some(page) = weak.upgrade()
                && let Some(window) = page.window.upgrade()
            {
                window.global::<LibraryCheck>().set_show_hidden(on);
                page.show();
            }
        });
        let weak = Rc::downgrade(&page);
        state.on_play(move |list, block, track| {
            if let Some(page) = weak.upgrade() {
                page.play(list, block, track);
            }
        });
        let weak = Rc::downgrade(&page);
        state.on_edit_tags(move |list, block, track| {
            if let Some(page) = weak.upgrade() {
                page.edit_tags(list, block, track);
            }
        });
        let weak = Rc::downgrade(&page);
        state.on_reveal(move |list, block, track| {
            if let Some(page) = weak.upgrade()
                && let Some(track) = page.track(list, block, track)
                && let Some(file) = &track.file
                && let Err(error) = file_manager::reveal(file)
            {
                log::warn!("could not show {}: {error}", file.display());
            }
        });
        let weak = Rc::downgrade(&page);
        state.on_not_duplicates(move |block| {
            if let Some(page) = weak.upgrade() {
                page.mark(block, true);
            }
        });
        let weak = Rc::downgrade(&page);
        state.on_show_again(move |block| {
            if let Some(page) = weak.upgrade() {
                page.mark(block, false);
            }
        });
        let weak = Rc::downgrade(&page);
        state.on_find_cover(move |index| {
            let Some(page) = weak.upgrade() else { return };
            let album = usize::try_from(index)
                .ok()
                .and_then(|index| page.coverless.borrow().get(index).map(|album| album.id));
            if let (Some(album), Some(window)) = (album, page.window.upgrade()) {
                window
                    .global::<CoverSearch>()
                    .invoke_find_album(album.0 as i32);
            }
        });
        page
    }

    /// Calls `play` with the tracks to play, the queue's context and where to start.
    pub fn on_play(&self, play: impl Fn(&[Track], Option<String>, usize) + 'static) {
        *self.on_play.borrow_mut() = Some(Box::new(play));
    }

    /// Reads the findings and shows the page, each tab from its start.
    pub fn open(&self) {
        self.shown.set([PAGE_SIZE; 3]);
        self.load();
        if let Some(window) = self.window.upgrade() {
            window.global::<LibraryCheck>().set_open(true);
        }
    }

    /// Reads the findings again after the library changed, while the page is open.
    pub fn reload(&self) {
        let open = self
            .window
            .upgrade()
            .is_some_and(|window| window.global::<LibraryCheck>().get_open());
        if open {
            self.load();
        }
    }

    fn load(&self) {
        let library = &self.library;
        *self.all_groups.borrow_mut() = read("duplicates", library.duplicate_groups());
        *self.gaps.borrow_mut() = read("missing tags", library.missing_tags());
        *self.coverless.borrow_mut() = read("missing covers", library.albums_without_cover());
        self.show();
    }

    /// Fills the global from what was read, as far as each tab lists.
    fn show(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<LibraryCheck>();
        let [duplicates_shown, gaps_shown, covers_shown] = self.shown.get();

        let all = self.all_groups.borrow();
        let hidden = all.iter().filter(|group| group.hidden).count();
        let groups: Vec<DuplicateGroup> = all
            .iter()
            .filter(|group| state.get_show_hidden() || !group.hidden)
            .cloned()
            .collect();
        state.set_duplicate_count(count(all.len() - hidden));
        state.set_hidden_count(count(hidden));
        state.set_more_duplicates(groups.len() > duplicates_shown);
        let blocks: Vec<DuplicateBlock> = groups
            .iter()
            .take(duplicates_shown)
            .map(duplicate_block)
            .collect();
        state.set_duplicates(model(blocks));
        *self.groups.borrow_mut() = groups;

        let gaps = self.gaps.borrow();
        state.set_gap_album_count(count(gaps.len()));
        state.set_gap_track_count(count(gaps.iter().map(|album| album.tracks.len()).sum()));
        state.set_more_gaps(gaps.len() > gaps_shown);
        let blocks: Vec<GapBlock> = gaps.iter().take(gaps_shown).map(gap_block).collect();
        state.set_gaps(model(blocks));

        let coverless = self.coverless.borrow();
        state.set_coverless_count(count(coverless.len()));
        state.set_more_coverless(coverless.len() > covers_shown);
        let albums: Vec<CoverlessAlbum> = coverless
            .iter()
            .take(covers_shown)
            .map(|album| CoverlessAlbum {
                title: album.title.as_str().into(),
                artist: album.artist.clone().unwrap_or_default().into(),
                year: album
                    .year
                    .map(|year| year.to_string())
                    .unwrap_or_default()
                    .into(),
                track_count: count(album.track_count as usize),
            })
            .collect();
        state.set_coverless(model(albums));
    }

    /// The track at `track` of block `block` in `list`.
    fn track(&self, list: CheckList, block: i32, track: i32) -> Option<Track> {
        let (block, track) = (usize::try_from(block).ok()?, usize::try_from(track).ok()?);
        match list {
            CheckList::Duplicates => self.groups.borrow().get(block)?.tracks.get(track).cloned(),
            CheckList::MissingTags => self
                .gaps
                .borrow()
                .get(block)?
                .tracks
                .get(track)
                .map(|gaps| gaps.track.clone()),
        }
    }

    /// A duplicate plays its group one after another, to compare them; a track
    /// missing tags plays its album from that track, as on the album page.
    fn play(&self, list: CheckList, block: i32, track: i32) {
        let Some(chosen) = self.track(list, block, track) else {
            return;
        };
        let (tracks, origin) = match list {
            CheckList::Duplicates => {
                let Some(group) = usize::try_from(block)
                    .ok()
                    .and_then(|block| self.groups.borrow().get(block).cloned())
                else {
                    return;
                };
                (group.tracks, None)
            }
            CheckList::MissingTags => {
                let Some(album) = chosen.album_id else { return };
                match self.library.album_tracks(album) {
                    Ok(tracks) => (tracks, Some(format!("album:{}", album.0))),
                    Err(error) => {
                        log::error!("could not read the tracks of album {}: {error}", album.0);
                        return;
                    }
                }
            }
        };
        let Some(start) = tracks.iter().position(|track| track.id == chosen.id) else {
            return;
        };
        if let Some(play) = self.on_play.borrow().as_ref() {
            play(&tracks, origin, start);
        }
    }

    /// Opens the tag editor for one track, or with `track` -1 for every track an
    /// album block of Missing tags lists.
    fn edit_tags(&self, list: CheckList, block: i32, track: i32) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let editor = window.global::<TagEditorState>();
        if track >= 0 {
            if let Some(track) = self.track(list, block, track) {
                editor.invoke_edit_track(track.key().into());
            }
            return;
        }
        let paths: Vec<SharedString> = match (list, usize::try_from(block)) {
            (CheckList::MissingTags, Ok(block)) => self
                .gaps
                .borrow()
                .get(block)
                .map(|album| {
                    album
                        .tracks
                        .iter()
                        .map(|gaps| gaps.track.key().into())
                        .collect()
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        if !paths.is_empty() {
            editor.invoke_edit_files(model(paths));
        }
    }

    /// Marks the group in block `block` as not duplicates, or undoes that.
    fn mark(&self, block: i32, not_duplicates: bool) {
        let ids: Vec<_> = match usize::try_from(block)
            .ok()
            .and_then(|block| self.groups.borrow().get(block).cloned())
        {
            Some(group) => group.tracks.iter().map(|track| track.id).collect(),
            None => return,
        };
        let result = if not_duplicates {
            self.library.mark_not_duplicates(&ids)
        } else {
            self.library.unmark_not_duplicates(&ids)
        };
        if let Err(error) = result {
            log::error!("could not store Not duplicates: {error}");
            return;
        }
        match self.library.duplicate_groups() {
            Ok(groups) => *self.all_groups.borrow_mut() = groups,
            Err(error) => log::error!("could not check the library for duplicates: {error}"),
        }
        self.show();
    }
}

/// What a check found, or nothing when it failed.
fn read<T>(what: &str, result: vcmp_library::Result<Vec<T>>) -> Vec<T> {
    result.unwrap_or_else(|error| {
        log::error!("could not check the library for {what}: {error}");
        Vec::new()
    })
}

fn tab_index(tab: CheckTab) -> usize {
    match tab {
        CheckTab::Duplicates => 0,
        CheckTab::MissingTags => 1,
        CheckTab::MissingCovers => 2,
    }
}

fn count(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

fn model<T: Clone + 'static>(rows: Vec<T>) -> ModelRc<T> {
    ModelRc::from(Rc::new(VecModel::from(rows)))
}

fn duplicate_block(group: &DuplicateGroup) -> DuplicateBlock {
    let first = group.tracks.first();
    DuplicateBlock {
        title: first
            .map(|track| track.title.as_str())
            .unwrap_or_default()
            .into(),
        artist: first
            .and_then(|track| track.artist.as_deref())
            .unwrap_or_default()
            .into(),
        hidden: group.hidden,
        tracks: model(
            group
                .tracks
                .iter()
                .map(|track| {
                    // Album, format and length tell the copies apart; they are data,
                    // and the separator is not a word to translate.
                    let details: Vec<String> = track
                        .album
                        .clone()
                        .into_iter()
                        .chain(track.format.map(|format| format.name().to_owned()))
                        .chain([format_duration(track.duration)])
                        .collect();
                    check_track(track, details.join(" · "), None)
                })
                .collect(),
        ),
    }
}

fn gap_block(album: &AlbumGaps) -> GapBlock {
    GapBlock {
        title: album.album.title.as_str().into(),
        artist: album.album.artist.clone().unwrap_or_default().into(),
        unknown: album.album.unknown,
        tracks: model(
            album
                .tracks
                .iter()
                .map(|gaps| {
                    let track = &gaps.track;
                    let details: Vec<String> = track
                        .artist
                        .clone()
                        .into_iter()
                        .chain([format_duration(track.duration)])
                        .collect();
                    check_track(track, details.join(" · "), Some(gaps))
                })
                .collect(),
        ),
    }
}

fn check_track(track: &Track, details: String, gaps: Option<&TrackGaps>) -> CheckTrack {
    let missing = gaps.map(|gaps| gaps.missing).unwrap_or_default();
    let path = track.key();
    CheckTrack {
        title: track.title.as_str().into(),
        number: track
            .track_number
            .map(|number| number.to_string())
            .unwrap_or_default()
            .into(),
        details: details.into(),
        path: path.as_str().into(),
        short_path: shorten_middle(&path, PATH_CHARS).into(),
        missing_title: missing.title,
        missing_artist: missing.artist,
        missing_album: missing.album,
        missing_number: missing.track_number,
        missing_year: missing.year,
    }
}

/// `text` with its middle given way to an ellipsis when it is longer than `max`
/// characters, keeping more of the end (a path's file name) than of the start.
fn shorten_middle(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max || max < 5 {
        return text.to_owned();
    }
    let keep = max - 1;
    let start = keep / 3;
    let end = keep - start;
    let mut shortened: String = chars[..start].iter().collect();
    shortened.push('…');
    shortened.extend(&chars[chars.len() - end..]);
    shortened
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_paths_lose_their_middle() {
        assert_eq!(shorten_middle("short", 10), "short");
        assert_eq!(shorten_middle("abcdefghijklmnop", 10), "abc…klmnop");
        assert_eq!(shorten_middle("abcdefghijklmnop", 10).chars().count(), 10);
    }
}
