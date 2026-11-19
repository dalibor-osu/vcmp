//! Settings' sections and its search (`ui/settings-nav.slint`). The page hands the
//! search its blocks, each with a title, its section and a few keywords; a block is
//! found when every typed word is in one of those, ignoring case, as the page
//! filters match. Links elsewhere (the setup wizard, the photo picker) open Settings
//! on a section through `open`.

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::navigation::section;
use crate::{
    AppWindow, LibraryCheck, Page, PhotoSearch, SettingEntry, SettingsNav, SettingsSection, Setup,
};

/// The sections in the order of `SettingsNav.sections`.
pub const SECTIONS: [SettingsSection; 6] = [
    SettingsSection::General,
    SettingsSection::Library,
    SettingsSection::Playback,
    SettingsSection::Appearance,
    SettingsSection::OnlineServices,
    SettingsSection::Agents,
];

/// What a search found: per block, per section, and how many blocks.
#[derive(Debug, PartialEq)]
pub struct Found {
    pub blocks: Vec<bool>,
    pub sections: [bool; 6],
    pub count: usize,
}

/// A block as the search reads it.
pub struct Block<'a> {
    pub section: SettingsSection,
    pub section_name: &'a str,
    pub title: &'a str,
    pub keywords: &'a str,
    pub available: bool,
}

/// Answers `SettingsNav.search` and `SettingsNav.open`.
pub fn install(window: &AppWindow) {
    let nav = window.global::<SettingsNav>();

    let weak = window.as_weak();
    nav.on_search(move |query, entries| {
        let Some(window) = weak.upgrade() else { return };
        let nav = window.global::<SettingsNav>();
        let entries: Vec<SettingEntry> = entries.iter().collect();
        let names: Vec<SharedString> = entries
            .iter()
            .map(|entry| nav.invoke_name(entry.section))
            .collect();
        let blocks: Vec<Block> = entries
            .iter()
            .zip(&names)
            .map(|(entry, name)| Block {
                section: entry.section,
                section_name: name,
                title: &entry.title,
                keywords: &entry.keywords,
                available: entry.available,
            })
            .collect();
        let found = find(&query, &blocks);
        nav.set_found(ModelRc::new(VecModel::from(found.blocks)));
        nav.set_found_sections(ModelRc::new(VecModel::from(found.sections.to_vec())));
        nav.set_found_count(i32::try_from(found.count).unwrap_or(i32::MAX));
    });

    let weak = window.as_weak();
    nav.on_open(move |settings_section| {
        let Some(window) = weak.upgrade() else { return };
        window.global::<Setup>().invoke_close();
        let photos = window.global::<PhotoSearch>();
        if photos.get_open() {
            photos.invoke_cancel();
        }
        window.global::<LibraryCheck>().set_open(false);
        window.global::<SettingsNav>().invoke_show(settings_section);
        window.invoke_show_section(section(Page::Settings));
    });
}

/// The blocks that hold every word of `query` in their title, their section's name
/// or their keywords, ignoring case. A block that isn't available is never found.
pub fn find(query: &str, blocks: &[Block]) -> Found {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut found = Found {
        blocks: Vec::with_capacity(blocks.len()),
        sections: [false; 6],
        count: 0,
    };
    for block in blocks {
        let names = [
            block.title.to_lowercase(),
            block.section_name.to_lowercase(),
            block.keywords.to_lowercase(),
        ];
        let matches = block.available
            && !words.is_empty()
            && words
                .iter()
                .all(|word| names.iter().any(|name| name.contains(word.as_str())));
        if matches {
            found.sections[index(block.section)] = true;
            found.count += 1;
        }
        found.blocks.push(matches);
    }
    found
}

/// The place of a section in `SECTIONS`.
pub fn index(settings_section: SettingsSection) -> usize {
    SECTIONS
        .iter()
        .position(|candidate| *candidate == settings_section)
        .unwrap_or(0)
}

/// The name a section is saved under.
pub fn saved_name(settings_section: SettingsSection) -> &'static str {
    match settings_section {
        SettingsSection::General => "general",
        SettingsSection::Library => "library",
        SettingsSection::Playback => "playback",
        SettingsSection::Appearance => "appearance",
        SettingsSection::OnlineServices => "online_services",
        SettingsSection::Agents => "agents",
    }
}

/// A saved section; `None` for a name this VCMP doesn't know.
pub fn parse_saved(name: &str) -> Option<SettingsSection> {
    SECTIONS
        .into_iter()
        .find(|candidate| saved_name(*candidate) == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block<'a>(section: SettingsSection, title: &'a str, keywords: &'a str) -> Block<'a> {
        Block {
            section,
            section_name: match section {
                SettingsSection::Appearance => "Appearance",
                SettingsSection::Playback => "Playback",
                _ => "Online services",
            },
            title,
            keywords,
            available: true,
        }
    }

    #[test]
    fn every_word_is_found_in_a_title_a_section_or_keywords() {
        let blocks = [
            block(SettingsSection::Appearance, "Theme", "dark light colours"),
            block(SettingsSection::Appearance, "Zoom", "scale size"),
            block(SettingsSection::Playback, "Shuffle", "random"),
        ];
        assert_eq!(find("zoom", &blocks).blocks, [false, true, false]);
        assert_eq!(
            find("DARK", &blocks).blocks,
            [true, false, false],
            "case is ignored"
        );
        assert_eq!(
            find("appear sca", &blocks).blocks,
            [false, true, false],
            "every word"
        );
        assert_eq!(
            find("appearance", &blocks).blocks,
            [true, true, false],
            "a section's name"
        );
        let found = find("random", &blocks);
        assert_eq!(found.count, 1);
        assert_eq!(found.sections, [false, false, true, false, false, false]);
    }

    #[test]
    fn nothing_is_found_without_words_or_for_unavailable_blocks() {
        let mut lastfm = block(SettingsSection::OnlineServices, "Last.fm", "scrobble");
        lastfm.available = false;
        let blocks = [
            lastfm,
            block(SettingsSection::Playback, "Shuffle", "random"),
        ];
        assert_eq!(find("scrobble", &blocks).count, 0);
        assert_eq!(find("  ", &blocks).blocks, [false, false]);
    }

    #[test]
    fn saved_sections_are_read_back() {
        for settings_section in SECTIONS {
            assert_eq!(
                parse_saved(saved_name(settings_section)),
                Some(settings_section)
            );
        }
        assert_eq!(parse_saved("someday"), None);
        assert_eq!(index(SettingsSection::Agents), 5);
    }
}
