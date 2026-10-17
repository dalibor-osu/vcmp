# UI foundation

How VCMP's Slint UI is put together and why. Visual direction comes from
[DESIGN.md](DESIGN.md); this file records how the code applies it. Keep it in sync
with `crates/vcmp-app/ui/`.

Design Read: a desktop music player shell for music enthusiasts, in a calm Catppuccin
pastel language with one subtle neon accent, dial ENERGY 1 / RHYTHM 2 / MOTION 2 (the
dials come from DESIGN.md).

## Files

| File | Holds |
|---|---|
| `ui/theme.slint` | `ThemePalette` struct, `ThemeMode` enum, `Theme` global: colours, spacing, radii, type sizes, target height, content width, durations |
| `ui/components.slint` | `FocusRing`, `NavItem`, `SegmentedChoice`, `Toggle`, `SettingRow` |
| `ui/pages.slint` | `SettingsPage`, `AboutPage` and their shared page frame |
| `ui/app-window.slint` | `AppWindow`: sidebar, current-page marker, content area |
| `src/theme.rs` | The light and dark palettes, plus their contrast tests |

## Rules enforced by tests

| Rule | Test |
|---|---|
| No colour literals in `.slint` files; every colour comes from `Theme.colors` | `tests/ui_invariants.rs` |
| Durations only in `theme.slint`, so reduced motion turns all motion off in one place | `tests/ui_invariants.rs` |
| Every user-visible string goes through `@tr()` (exceptions: "VCMP", "Sora", the slint.dev URL) | `tests/ui_invariants.rs` |
| Every text pairing reaches 4.5:1 and every control boundary 3:1, in both palettes | `src/theme.rs` |
| Clicking, keyboard-only use, theme choice and reduced motion behave as designed; nothing leaves a 720 px wide window | `tests/ui_behaviour.rs` |

## Palettes

Catppuccin Latte (light) and Mocha (dark), with Pink as the single accent.

| Token | Light | Dark | Use |
|---|---|---|---|
| base | `#eff1f5` | `#1e1e2e` | Content background |
| mantle | `#e6e9ef` | `#181825` | Sidebar |
| surface | `#ccd0da` | `#313244` | Hover, selected item, control track |
| outline | `#7c7f93` (overlay2) | `#6c7086` (overlay0) | Control boundaries |
| text | `#4c4f69` | `#cdd6f4` | Text |
| text-muted | `#5c5f77` (subtext1) | `#a6adc8` (subtext0) | Descriptions, only on base or mantle |
| accent | `#a1197d` | `#f5c2e7` (pink) | Current page, selected option, switched-on state |
| on-accent | `#eff1f5` | `#11111b` | Text and marks on accent |
| glow | `#ea76cb` at 69% | `#f5c2e7` at 69% | Neon glow around the page marker |

Latte's own pink (`#ea76cb`) reaches only 2.3:1 on Latte base, so the light accent is
the same hue darkened to 6.3:1. The original pink survives as the light glow. Latte's
subtext0 and overlay0 fail contrast on its backgrounds, so the light theme uses
subtext1 and overlay2 instead.

## Design decisions

| Decision | Reason |
|---|---|
| Palettes live in Rust, not in Slint | One source for the UI and the contrast tests; future custom themes only swap values |
| Pink marks what is active (page, option, switch) | Gives the neon accent a job, so it stays an accent rather than decoration |
| Glow only on the current-page marker | DESIGN.md asks for subtle neon; one glowing element per screen keeps it subtle |
| Sora for everything, two weights (400, 600) | The owner's choice; two weights carry hierarchy without a second family at ENERGY 1 |
| Type scale 12 / 14 / 16 / 20 / 26 px | Small steps suit a calm UI; the page title is the one large element per screen |
| Spacing scale 4 / 8 / 12 / 16 / 24 / 40 px | Bigger gaps separate sections, small ones group a label with its description |
| Radii 2 / 8 / 12 px, pill only for the switch | The switch track is the one shape where a pill carries meaning |
| Content column capped at 720 px | Keeps a setting's label and its control within one glance on wide windows |
| Sidebar plus content area | Agreed in the spec; it holds only the pages that exist (Settings, About) |
| Light and dark plus "System", defaulting to System | No brand reason to force a theme; following the OS is least surprising |
| Motion: marker slide, page fade, control transitions (120 / 220 ms) | MOTION 2: transitions show where the selection went; nothing loops |

## Accessibility patterns

- Every control is a `FocusScope` with Space or Enter (arrows in `SegmentedChoice`)
  and an accent `FocusRing` drawn 3 px outside it. Mouse clicks do not move focus
  (`focus-on-click: false`), so the ring appears only for keyboard use.
- Controls set `accessible-role`, `accessible-label`, the checked or selected state,
  and `accessible-action-default`. Tests find elements through these properties.
- Interactive targets are at least 40 px tall (`Theme.target-height`).
- State is never shown by colour alone: the selected option is bold and filled, and
  the switch knob changes side.

## Known limits

- The Slint attribution (`AboutSlint`) picks its logo from the operating system's
  colour scheme, not VCMP's theme, so it sits on the system palette's background. It
  can look out of place when VCMP's theme differs from the OS.
- Theme and reduced-motion choices last only until the app closes. They will be
  saved once the library database exists (workstream 2).
- Reduced motion does not yet follow the OS preference. The spec asks for that where
  a platform exposes one; it needs per-platform code.
- `examples/screenshots.rs` cannot force a window size under tiling window managers.
  Layout at the smallest window size is covered by a test instead.
