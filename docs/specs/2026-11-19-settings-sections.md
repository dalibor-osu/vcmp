# Settings sections

Status: agreed
Date: 2026-11-19
Area: vcmp-app (Settings page, its navigation and back/forward, the setup wizard's and photo picker's pointers into Settings, the tests and screenshots that reach Settings rows)

## Context of the change

Settings (`SettingsPage` in `crates/vcmp-app/ui/pages.slint`) is one long scrolling
column, 60 % of the page between 720 and 960 px (`Theme.column-max-width`), centred,
as the owner asked on 2026-10-20 (`docs/DESIGN.md`, `docs/ui.md` "Settings and About
in a column"). It has grown with every feature and now holds, top to bottom:

| Today's heading | Rows |
|---|---|
| (none) | Setup (Run setup again) |
| Music folders | Folder list, Add folder, scan progress (`LibrarySection`, `library-settings.slint`) |
| (none) | Library check (opens `LibraryCheckPage` in place of Settings), Update automatically |
| Playback | Shuffle, Output device, Show on Discord, Find lyrics online, Last.fm (connect, love, import loved tracks), Play history |
| Artist photos | fanart.tv personal key (only in builds with fanart.tv) |
| Appearance | Theme, Accent, Theme files, Zoom, Reduce motion |
| Labels | The labels list (`LabelsSection`, `labels.slint`) |
| AI agents | Switch, connection, playback switch, port, changes by agents, backups (`AgentsSection`, `agents.slint`) |
| Updates | Check for updates (`UpdatesSection`, `updates.slint`) |

So there are headings already, but nothing to move between them: finding a setting
means scrolling the whole page, and some rows have no heading or sit under one that
does not describe them (Playback holds Discord, lyrics, Last.fm and play history).

Related pieces the change has to respect:

- The setup wizard (`setup.slint`) reuses Settings' own row components
  (`LibrarySection`, `LastfmRow`, `DiscordRow`, `LyricsOnlineRow`, `ThemeRows`,
  `ZoomRow`, `MotionRow`, `UpdateCheckRow`, `PhotoKeyRow`, `AgentsSwitchRow`), so
  every choice applies at once (`docs/ui.md`, setup wizard).
- Back and forward (`src/navigation.rs`) treat each sidebar section, each open
  album, artist, playlist or label, and the Library check as a `Place`.
- Several texts point into Settings: the wizard's "the token and the setup commands
  are under AI agents" and "Run setup again, at the top of Settings", the photo
  picker's "fanart.tv didn't accept the personal API key from Settings", and the
  Library check's back link labelled "Settings".
- Tests (`tests/ui_behaviour.rs`, `preferences.rs`, `agents.rs`, `lastfm.rs`,
  `labels.rs`, `play_history.rs` and others) and `examples/screenshots.rs` reach
  Settings rows by opening the page in a window tall enough for the whole page, or
  far enough down. `everything_works_from_the_keyboard` walks the Tab order from the
  Settings entry straight into "Run setup again", Add folder and the rows below.
- `docs/ui.md` rules hold for any new UI: no colour literals or durations outside
  `theme.slint`, every visible string in `@tr()`, WCAG AA in every built-in theme,
  everything operable by keyboard.

The owner filed issue #108 ("Settings improvements", high priority) because there is
quite a lot in Settings now.

## User request

Split the Settings view into sections, since there is quite a lot in it now (issue
#108, "Settings improvements"). Brainstorm the feature described in the issue and
create a specification for it, in a new worktree on a new feature branch.

## Decisions

### What shape should the split of Settings take?

| Option | What it means |
|---|---|
| Section list + one section (Recommended) | A short list of section names beside the content, one section shown at a time; short pages and clear place, but needs a narrow-window form and tests must open a section first. |
| Tabs across the top | A row of tabs above the content, one section at a time; familiar, but 7-9 tabs don't fit a 720 px column without wrapping or scrolling. |
| One page with jump links | Keep the single scrolling page and add a list that jumps to each heading; least change and nothing hidden, but the page stays as long as now. |
| Sub-entries in the sidebar | Settings' sections listed under Settings in the main sidebar; reuses the nav, but lengthens the sidebar and changes the owner's agreed sidebar layout. |

**Answer: Section list + one section (Recommended)**

### Besides splitting, may this change also regroup and rename rows (e.g. move Discord, lyrics, Last.fm and Play history out of Playback; give Setup, Library check and Update automatically a section)?

| Option | What it means |
|---|---|
| Split and regroup (Recommended) | Sections follow what the rows are about, which the split needs to read well; touches more rows and the wizard's shared ones keep working. |
| Split only, keep groups | Today's headings become the sections as they are; smallest change, but Playback stays a mixed bag and headless rows need a home anyway. |
| Split, regroup, and more | Also add further Settings improvements in the same change (you name them); bigger scope and spec. |

**Answer: Split and regroup (Recommended)**

### Which list of sections, and which rows in each?

| Option | What it means |
|---|---|
| 7 sections (Recommended) | General (Run setup again, Check for updates); Library (Music folders, Update automatically, Library check, Play history); Playback (Shuffle, Output device); Appearance (Theme, Accent, Theme files, Zoom, Reduce motion); Online services (Find lyrics online, Show on Discord, Last.fm connect, love and import, fanart.tv key when the build has it); Labels; AI agents. Each section stays short, the list is a little longer. |
| 6 sections, Labels in Library | As above, but the labels list joins Library. Labels sit with the music they tag; one fewer entry, but Library gets long once there are many labels. |
| 7, listening rows in Playback | Last.fm, Discord and Play history stay in Playback as things about listening; an "Online lookups" section keeps only Find lyrics online and the fanart.tv key, matching less with the wizard's Online services step. |

**Answer: 6 sections, Labels in Library** - the sections are:

| Section | Rows |
|---|---|
| General | Run setup again, Check for updates |
| Library | Music folders, Update automatically, Library check, Play history, Labels |
| Playback | Shuffle, Output device |
| Appearance | Theme, Accent, Theme files, Zoom, Reduce motion |
| Online services | Find lyrics online, Show on Discord, Last.fm (connect, love, import loved tracks), fanart.tv personal key (when the build has fanart.tv) |
| AI agents | Switch, connection, playback switch, port, changes by agents, backups for undo |

### Which section shows when Settings is opened from the sidebar?

| Option | What it means |
|---|---|
| Last shown, kept on restart (Recommended) | Goes back to where you were, even after restarting; saves clicks, one more value in the saved settings. |
| Last shown, General on start | Remembered while VCMP runs, General after a restart; nothing saved, but a restart loses the place. |
| Always General | Matches the rule that a sidebar entry always shows its full view; predictable, but often one extra click. |

**Answer: Last shown, kept on restart (Recommended)**

### Is switching between Settings sections part of back/forward (mouse buttons, Alt+Left/Right)?

| Option | What it means |
|---|---|
| Yes, each section a place (Recommended) | Back returns to the previous section, like the Library check already does; consistent, but a few section clicks fill the history. |
| No, pages only | Back leaves Settings for the previous page; history stays short, but Back from the Library check's section jump feels uneven. |

**Answer: Yes, each section a place (Recommended)**

### Where does the section list sit, and what happens in a narrow window (the page can be about 500 px wide)?

| Option | What it means |
|---|---|
| Left list, dropdown when narrow (Recommended) | A fixed list left of the section, which scrolls on its own; below a breakpoint the list becomes a dropdown above the section. Two forms to build and test. |
| Left list, always | The list stays beside the section at every width; one form only, but the section gets cramped in a narrow window. |
| Left list, icons only when narrow | The list shrinks to icons in a narrow window; keeps the place visible, but needs an icon per section and tooltips for the names. |

**Answer: Left list, dropdown when narrow (Recommended)**

### Should the places that point into Settings open the right section directly (the wizard's 'token is under AI agents' line, the fanart.tv key error, the Library check's back link)?

| Option | What it means |
|---|---|
| Back link only, text updated (Recommended) | The Library check's back returns to Library; other texts name the section ('Settings > AI agents') but stay plain text. Small, no new links in dialogs. |
| Make them links | Each pointer opens Settings on its section (closing the wizard or picker); handier, but adds links and focus handling in dialogs. |
| Back link only, text as is | Only the Library check returns to Library; the texts keep saying 'Settings', which stays true but less exact. |

**Answer: Make them links**

### A search box that finds a setting across sections: part of this change or later?

| Option | What it means |
|---|---|
| Later (Recommended) | Six short sections are easy to scan; keeps this change to the split, search can come if Settings keeps growing. |
| In this change | Typing finds a row in any section and opens it; helps as Settings grows, but needs a searchable index of every row and more tests. |

**Answer: In this change**

### How does the Settings search show what it finds?

| Option | What it means |
|---|---|
| Result list, opens the row (Recommended) | A field above the section list; typing shows matching rows by name with their section, choosing one opens that section with the row in view and focused. Simple, but a second step to change the value. |
| Live rows across sections | Typing shows the matching rows themselves, working, from every section; fastest to change a value, but rows that depend on their section (Last.fm states, agents' connection) look odd out of context. |
| Narrow the section list | Typing hides sections without a match and highlights matches inside; least new UI, but still a click into the section and weak with one-word queries. |

**Answer: Live rows across sections**

### What does the search match?

| Option | What it means |
|---|---|
| Titles, section, keywords (Recommended) | Row titles, section names and a few hand-picked words per row (e.g. 'dark' finds Theme, 'scale' finds Zoom); precise, but the keywords are a list to keep up. |
| Titles and descriptions | Also the sentence under each row; finds more with no extra list, but common words match many rows. |
| Titles only | Smallest and exact, but misses anything not named the way you type it. |

**Answer: Titles, section, keywords (Recommended)**

## High-level plan

**1. Sections and their frame.** Settings gets a page title, a search field and a
list of the six sections (General, Library, Playback, Appearance, Online services,
AI agents) on the left of the column, with the chosen section's rows beside it. The
list stays in place while the section scrolls. Below a width breakpoint the list
turns into a dropdown above the section. The rows are regrouped into the sections
decided above; their components stay the ones the setup wizard shares, so the wizard
keeps working unchanged. Every part is reachable and operable by keyboard. Depends
on nothing.

**2. Section state and navigation.** The shown section is saved in the library's
settings and restored at start; an unknown saved value falls back to General. Each
section becomes a place in back/forward. The Library check is opened from Library
and returns to it. Depends on 1.

**3. Pointers into Settings.** A way to open Settings on a given section from
anywhere: the wizard's line about the agents' token links to AI agents and closes
the wizard, the photo picker's key error links to Online services and closes the
picker, the Library check's back link returns to Library. Texts that place things
("at the top of Settings") name the section instead. Depends on 2.

**4. Settings search.** The field above the list matches each typed word against row
titles, section names and a few keywords per row. While it holds text, the content
area shows the matching rows themselves, working, from every section, and says so
when nothing matches. Rows hidden by the build or the current state stay hidden in
results. Depends on 1; can be built alongside 2 and 3.

**5. Tests, screenshots and documentation.** Tests and the screenshots example open
a section before reaching a row, and the keyboard test follows the new Tab order;
new tests cover section switching, the saved section, back/forward, the narrow form,
the links and search. The UI documentation, the owner's design record (the decisions
in this spec) and the manual testing list (what tests can't see, such as the narrow
form at real zoom levels) are updated in the same change. Comes last but follows
each phase.

## Architecture decisions

| Decision | Reason it beat the alternative |
|---|---|
| Sections are composed in `SettingsPage` from the existing row components; no row component moves into a section-specific wrapper the wizard can't use | The wizard and Settings must keep sharing controls so a choice applies once (`docs/ui.md`). Regrouping in the page only leaves the wizard untouched |
| The section is a Slint enum on a Settings global, set from the list, the dropdown, links and Rust | One value drives the list, the dropdown, the content, saving and history; links from the wizard and picker set it without knowing the page |
| `navigation.rs` gets a Settings place carrying the section, beside the existing `LibraryCheck` place | The history already models "what Settings shows"; adding the section there keeps "every way of moving around is remembered without each knowing about the history" |
| The saved section is one key in the library's settings, next to the other UI preferences | Same store and lifetime as the track columns and other remembered UI state; no new storage |
| The search index is one table of entries `{row id, section, title, keywords}` built in Slint with `@tr()` strings and handed to Rust; Rust matches by the same rule as the page filters (every typed word, lowercased, `library_view.rs`) and returns the set of matching row ids | The page filters already match in Rust, so search follows the same rule in the same place; keeping titles and keywords in Slint keeps one translated source for each title instead of copying them into Rust |
| The unit of search is a block as shown in Settings: a single row, or a composite block (the music folder list, the labels list, the agents' connection) shown whole | Composite blocks only make sense whole; splitting them would mean new components the wizard doesn't share |
| Components that bundle several rows (`AgentsSection`: switch, connection, playback switch, Port, changes, Backups; `ThemeRows`: Theme, Accent, Theme files) expose their rows individually to the page, and section components lose their own headings (`AgentsSection`, `LabelsSection`, `UpdatesSection`), since the section list names the section | Otherwise Port, Backups or Accent could only be found as their whole bundle, and headings inside a section would repeat its name. The wizard's shared rows (`AgentsSwitchRow`, `ThemeRows` in its wizard form) keep working as they are |
| A row's visibility in results is its index match AND its own existing condition (build availability, connected state, switch on) | Search must never surface a control the section view would hide |
| A test enforces that every block in Settings has an index entry and every index entry has a block | Otherwise a row added later silently drops out of search; the repo already enforces UI rules in `tests/ui_invariants.rs` style tests |
| The narrow form switches on the column's measured width, not the window's | Zoom and the queue panel change the room Settings has independently of the window size |

## Weaknesses and risks

| Risk | Likelihood | Cost | What reduces it |
|---|---|---|---|
| Settings now hides most rows; a user looking for a setting picks the wrong section | Medium | A few extra clicks | Search across sections, the remembered section, section names that match the wizard's steps |
| Live rows out of context in search results look odd (Last.fm love without the connect row above it, agents' connection without the switch) | High | Confusing results | Show results under their section name; a composite block appears whole; state-dependent rows keep their conditions. The owner chose this knowing the trade-off; revisit after use |
| The search index drifts from the rows (row added, entry forgotten; title changed in one place only) | Medium | A setting that can't be found | The invariant test both ways; titles taken from one `@tr()` source |
| Keywords are English only in v1 and need translating with the rest | Certain once a language is added | Weaker search in other languages | Keywords go through `@tr()` like every string, so `slint-tr-extractor` picks them up |
| Library gets long with many labels plus folders, check, history | Medium for heavy label users | Scrolling inside one section | The owner chose it; search finds the Labels block (not a label by name); can be split later if it grows |
| Opening Settings on the last shown section is an exception to the owner's 2026-10-18 rule that a sidebar entry always goes to its full view | Certain | A rule with one exception | The owner chose it with "Always General" on the table; DESIGN.md records it as an exception |
| Splitting `AgentsSection` and `ThemeRows` into separately visible rows touches components the wizard uses | Medium | A wizard regression | The wizard's tests and screenshots run in the same change |
| Two navigation forms (list and dropdown) double the UI to test and keep accessible | Certain | Test and screenshot effort | Shared state on one global; screenshots of both forms; tests at both widths |
| Back/forward fills with section clicks | Medium | Longer history | The existing 100-place limit; consecutive identical places are not repeated |
| Links from the wizard and photo picker close a dialog the user was in | Low | Lost context: the wizard's step, the picker's search | Leaving the wizard early already keeps every choice; the picker can be opened again from the artist page |
| Tests that reach rows by scrolling break broadly | Certain | One-off effort across many test files | A shared test helper that opens a section; done in the same change |
| A saved section from a newer or older build is unknown | Low | Settings opens somewhere odd | Fall back to General |

Left open, to settle while building: the exact breakpoint width for the dropdown, the
keyword list per row, and whether the search field also answers Ctrl+F on the
Settings page as the filters do on other pages. None of them changes the shape
above.

Not chosen by the owner, so they go to DESIGN.md's "Awaiting the owner's review" when
built: the list styling and the marker of the current section (following the
sidebar's own), whether search results are grouped under their section names, what
the section list shows while a search is active, whether choosing a section clears
the query, and whether a search is a place in back/forward.

## Out of scope

- The main Search page (Ctrl+F from other pages) does not find settings; the search
  here is only on the Settings page.
- No new settings, and no change to what any existing row does.
- The setup wizard's steps and their content stay as they are, except for the line
  about the agents' token becoming a link.
- About stays its own sidebar page; it does not become a Settings section.
- No settings import or export, and no reset-to-defaults.
- Search does not match the descriptions under rows, only titles, section names and
  keywords.
