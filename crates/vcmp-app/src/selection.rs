//! Multiple selection in the track lists and the songs table. Each row holds whether
//! it is selected, so the selection lives as long as the list's model and goes with
//! it; Slint can't loop over rows, so ranges, "all" and "none" are applied here.
//! The menu's actions take the list and the row the menu is for: they act on the
//! selected rows when that row is one of them, or else on that row alone.

use slint::{ComponentHandle, Model, ModelRc, SharedString};

use crate::{AppWindow, SelectionChange, SongRow, SongSelection, TrackRow, TrackSelection};

/// A row that can be selected and names its file.
pub trait Selectable: Clone {
    fn selected(&self) -> bool;
    fn set_selected(&mut self, selected: bool);
    fn path(&self) -> &SharedString;
}

impl Selectable for TrackRow {
    fn selected(&self) -> bool {
        self.selected
    }
    fn set_selected(&mut self, selected: bool) {
        self.selected = selected;
    }
    fn path(&self) -> &SharedString {
        &self.path
    }
}

impl Selectable for SongRow {
    fn selected(&self) -> bool {
        self.selected
    }
    fn set_selected(&mut self, selected: bool) {
        self.selected = selected;
    }
    fn path(&self) -> &SharedString {
        &self.path
    }
}

/// Applies a selection change to `rows`: row `index` toggled, the rows from
/// `anchor` to `index` and no others, all of them, or none. Only rows whose state
/// changes are written. Returns how many rows are selected afterwards.
pub fn change<T: Selectable + 'static>(
    rows: &ModelRc<T>,
    index: i32,
    anchor: i32,
    change: SelectionChange,
) -> i32 {
    let count = rows.row_count();
    let clamp = |n: i32| (n.max(0) as usize).min(count.saturating_sub(1));
    let (from, to) = {
        let (a, b) = (clamp(anchor), clamp(index));
        (a.min(b), a.max(b))
    };
    let mut selected = 0;
    for row in 0..count {
        let Some(mut data) = rows.row_data(row) else {
            continue;
        };
        let now = match change {
            SelectionChange::Toggle if row == clamp(index) => !data.selected(),
            SelectionChange::Toggle => data.selected(),
            SelectionChange::Range => (from..=to).contains(&row),
            SelectionChange::All => true,
            SelectionChange::None => false,
        };
        if now != data.selected() {
            data.set_selected(now);
            rows.set_row_data(row, data);
        }
        selected += i32::from(now);
    }
    selected
}

/// The rows a menu for row `index` acts on: every selected row, in list order, when
/// `index` is one of them; otherwise `index` alone (if it exists).
pub fn chosen<T: Selectable>(rows: &ModelRc<T>, index: usize) -> Vec<usize> {
    match rows.row_data(index) {
        Some(row) if row.selected() => (0..rows.row_count())
            .filter(|&n| rows.row_data(n).is_some_and(|row| row.selected()))
            .collect(),
        Some(_) => vec![index],
        None => Vec::new(),
    }
}

/// The files of the rows `chosen` picks.
pub fn chosen_paths<T: Selectable>(rows: &ModelRc<T>, index: i32) -> Vec<String> {
    let Ok(index) = usize::try_from(index) else {
        return Vec::new();
    };
    chosen(rows, index)
        .into_iter()
        .filter_map(|n| rows.row_data(n))
        .map(|row| row.path().to_string())
        .collect()
}

/// Lets the lists change their selection. The actions on the selection are
/// connected by the parts that carry them out (playlists, the tag editor).
pub fn install(window: &AppWindow) {
    window
        .global::<TrackSelection>()
        .on_change(|rows, index, anchor, how| change(&rows, index, anchor, how));
    window
        .global::<SongSelection>()
        .on_change(|rows, index, anchor, how| change(&rows, index, anchor, how));
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use slint::VecModel;

    use super::*;

    fn rows(count: usize) -> ModelRc<TrackRow> {
        let rows: Vec<TrackRow> = (0..count)
            .map(|n| TrackRow {
                path: format!("/m/{n}.flac").into(),
                ..TrackRow::default()
            })
            .collect();
        ModelRc::from(Rc::new(VecModel::from(rows)))
    }

    fn selected(rows: &ModelRc<TrackRow>) -> Vec<usize> {
        (0..rows.row_count())
            .filter(|&n| rows.row_data(n).unwrap().selected)
            .collect()
    }

    #[test]
    fn toggles_ranges_all_and_none() {
        let list = rows(6);
        assert_eq!(change(&list, 1, 1, SelectionChange::Toggle), 1);
        assert_eq!(change(&list, 4, 4, SelectionChange::Toggle), 2);
        assert_eq!(selected(&list), [1, 4]);
        // A range replaces the selection, in either direction from its anchor.
        assert_eq!(change(&list, 2, 4, SelectionChange::Range), 3);
        assert_eq!(selected(&list), [2, 3, 4]);
        assert_eq!(change(&list, 5, 4, SelectionChange::Range), 2);
        assert_eq!(selected(&list), [4, 5]);
        assert_eq!(change(&list, 4, 4, SelectionChange::Toggle), 1);
        assert_eq!(change(&list, 0, 0, SelectionChange::All), 6);
        assert_eq!(change(&list, 0, 0, SelectionChange::None), 0);
        assert!(selected(&list).is_empty());
        // Out of range indices are held to the list.
        assert_eq!(change(&list, 99, -3, SelectionChange::Range), 6);
        assert_eq!(change(&rows(0), 0, 0, SelectionChange::All), 0);
    }

    #[test]
    fn a_menu_acts_on_the_selection_it_was_opened_on() {
        let list = rows(5);
        change(&list, 1, 1, SelectionChange::Toggle);
        change(&list, 3, 3, SelectionChange::Toggle);
        assert_eq!(chosen(&list, 3), [1, 3]);
        assert_eq!(chosen(&list, 2), [2], "a row outside it acts alone");
        assert_eq!(chosen(&list, 9), Vec::<usize>::new());
        assert_eq!(chosen_paths(&list, 1), ["/m/1.flac", "/m/3.flac"]);
        assert!(chosen_paths(&list, -1).is_empty());
    }
}
