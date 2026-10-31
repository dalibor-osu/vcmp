//! Source checks for the rules in ui/theme.slint and the localization policy: colours
//! and durations come only from the Theme global, and user-visible text goes through
//! `@tr()`. Also a Slint 1.18.1 workaround every wrapped text needs, and no scroll
//! bars.

use std::fs;
use std::path::{Path, PathBuf};

/// String literals that are not translatable text: the product name, the font family,
/// a URL, and the empty string (used in comparisons).
// Names, the font, a URL, the empty string, and the keys in the Ctrl+F and Ctrl+A
// shortcuts.
const ALLOWED_LITERALS: &[&str] = &[
    "\"VCMP\"",
    "\"Sora\"",
    "\"https://slint.dev\"",
    "\"https://fanart.tv\"",
    "\"https://fanart.tv/get-an-api-key/\"",
    "\"https://coverartarchive.org\"",
    "\"https://musicbrainz.org\"",
    "\"\"",
    "\"f\"",
    "\"F\"",
    "\"a\"",
    "\"A\"",
    // The zoom shortcuts' keys.
    "\"=\"",
    "\"+\"",
    "\"-\"",
    "\"0\"",
];

fn slint_sources() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("ui directory")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "slint"))
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "no .slint files found in {}",
        dir.display()
    );
    files
}

/// The line up to a `//` comment that is not inside a string literal.
fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    let mut previous = '\0';
    for (index, c) in line.char_indices() {
        match c {
            '"' if previous != '\\' => in_string = !in_string,
            '/' if !in_string && previous == '/' => return &line[..index - 1],
            _ => {}
        }
        previous = c;
    }
    line
}

/// Code lines of a file, without `//` comments and import statements.
fn code_lines(path: &Path) -> Vec<(usize, String)> {
    let source = fs::read_to_string(path).expect("readable .slint file");
    source
        .lines()
        .enumerate()
        .map(|(index, line)| (index + 1, strip_comment(line).trim().to_string()))
        .filter(|(_, line)| !line.is_empty() && !line.starts_with("import "))
        .collect()
}

fn is_theme_file(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == "theme.slint")
}

#[test]
fn no_colour_literals_in_ui() {
    let mut violations = Vec::new();
    for path in slint_sources() {
        for (number, line) in code_lines(&path) {
            let has_hex = line.char_indices().any(|(i, c)| {
                c == '#'
                    && line[i + 1..]
                        .chars()
                        .take_while(char::is_ascii_hexdigit)
                        .count()
                        >= 3
            });
            let has_function = ["rgb(", "rgba(", "hsv(", "oklch(", "Colors."]
                .iter()
                .any(|f| line.contains(f));
            if has_hex || has_function {
                violations.push(format!("{}:{number}: {line}", path.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "colours must come from Theme.colors:\n{}",
        violations.join("\n")
    );
}

#[test]
fn durations_only_in_theme() {
    let mut violations = Vec::new();
    for path in slint_sources()
        .into_iter()
        .filter(|path| !is_theme_file(path))
    {
        for (number, line) in code_lines(&path) {
            let has_duration = line
                .split(|c: char| !c.is_ascii_alphanumeric())
                .any(|word| {
                    word.len() > 2
                        && word.ends_with("ms")
                        && word[..word.len() - 2].chars().all(|c| c.is_ascii_digit())
                });
            if has_duration {
                violations.push(format!("{}:{number}: {line}", path.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "durations must come from Theme:\n{}",
        violations.join("\n")
    );
}

/// Nothing scrolls in the std-widgets `ScrollView`, which draws its bar over the
/// content and widens it on hover; `ScrollArea` (components.slint) scrolls without
/// one (docs/ui.md, "No scroll bars anywhere").
#[test]
fn no_scroll_bars() {
    let mut violations = Vec::new();
    for path in slint_sources() {
        let source = fs::read_to_string(&path).expect("readable .slint file");
        for (index, line) in source.lines().enumerate() {
            let code = strip_comment(line);
            let mut words = code.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'));
            if words.any(|word| word == "ScrollView") {
                violations.push(format!("{}:{}: {}", path.display(), index + 1, code.trim()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "scroll in a ScrollArea, not a ScrollView:\n{}",
        violations.join("\n")
    );
}

/// Corner radii are `Theme` tokens, or worked out from the element's own size (half
/// of it for round shapes: icon buttons, tracks, knobs, drop lines); never a bare
/// pixel value.
#[test]
fn radii_come_from_theme_or_the_shape() {
    let mut violations = Vec::new();
    for path in slint_sources()
        .into_iter()
        .filter(|path| !is_theme_file(path))
    {
        for (number, line) in code_lines(&path) {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.trim().trim_end_matches(';');
            let bare = value
                .strip_suffix("px")
                .is_some_and(|number| number.parse::<f32>().is_ok());
            if name.trim().ends_with("radius") && bare {
                violations.push(format!("{}:{number}: {line}", path.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "radii must come from Theme or the element's size:\n{}",
        violations.join("\n")
    );
}

/// Removes every `@tr(...)` call, including plural forms such as
/// `@tr("{n} track" | "{n} tracks" % count)`, and every `@image-url(...)` (a file
/// name), leaving the rest of the line.
fn without_tr_calls(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    loop {
        let next = ["@tr(", "@image-url("]
            .iter()
            .filter_map(|call| rest.find(call).map(|start| (start, call.len())))
            .min();
        let Some((start, opening)) = next else { break };
        out.push_str(&rest[..start]);
        let call = &rest[start + opening..];
        let (mut depth, mut in_string, mut previous, mut end) = (1, false, '\0', call.len());
        for (index, c) in call.char_indices() {
            match c {
                '"' if previous != '\\' => in_string = !in_string,
                '(' if !in_string => depth += 1,
                ')' if !in_string => {
                    depth -= 1;
                    if depth == 0 {
                        end = index + 1;
                        break;
                    }
                }
                _ => {}
            }
            previous = c;
        }
        rest = &call[end..];
    }
    out.push_str(rest);
    out
}

/// Removes string literals that are SVG path data (icon shapes such as
/// "M 7 4 L 20 12 Z"): drawing commands and numbers, not text.
fn without_path_data(line: &str) -> String {
    let is_path_data = |text: &str| {
        text.starts_with('M')
            && text
                .chars()
                .all(|c| "MLHVCQZmlhvcqz0123456789 .,-".contains(c))
    };
    let mut out = String::new();
    let mut rest = line;
    while let Some(start) = rest.find('"') {
        let Some(length) = rest[start + 1..].find('"') else {
            break;
        };
        let literal = &rest[start..start + length + 2];
        out.push_str(&rest[..start]);
        if !is_path_data(&literal[1..literal.len() - 1]) {
            out.push_str(literal);
        }
        rest = &rest[start + length + 2..];
    }
    out.push_str(rest);
    out
}

#[test]
fn text_is_translatable() {
    let mut violations = Vec::new();
    for path in slint_sources() {
        for (number, line) in code_lines(&path) {
            let mut remaining = without_path_data(&without_tr_calls(&line));
            for literal in ALLOWED_LITERALS {
                remaining = remaining.replace(literal, "");
            }
            if remaining.contains('"') {
                violations.push(format!("{}:{number}: {line}", path.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "user-visible text must use @tr():\n{}",
        violations.join("\n")
    );
}

/// Slint 1.18.1 measures the height of an element an `if` creates at its single-line
/// width, so a wrapped text there, or in a layout there, lost its last lines
/// (slint-ui/slint#13655, fixed after 1.18.1). A wrapped text's own preferred height
/// follows its real width, so every wrapped text asks for at least that, unless it
/// sets its height outright. Drop this rule once Slint is past the fix.
#[test]
fn wrapped_text_asks_for_its_own_height() {
    let mut missing = Vec::new();
    for path in slint_sources() {
        let source = fs::read_to_string(&path).expect("read source");
        let lines: Vec<&str> = source.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            let code = strip_comment(line).trim();
            if !code.starts_with("wrap:") || !code.contains("word-wrap") {
                continue;
            }
            // The element: from its opening line, at one indent less, to its close.
            let indent = line.len() - line.trim_start().len();
            let Some(open) = (0..index).rev().find(|&i| {
                let other = lines[i];
                other.len() - other.trim_start().len() + 4 == indent
                    && other.trim_end().ends_with('{')
            }) else {
                continue;
            };
            let mut depth = 0i32;
            let mut sized = false;
            for other in &lines[open..] {
                let code = strip_comment(other).trim();
                if code.starts_with("min-height: self.preferred-height")
                    || code.starts_with("height:")
                {
                    sized = true;
                }
                depth += other.matches('{').count() as i32 - other.matches('}').count() as i32;
                if depth == 0 {
                    break;
                }
            }
            if !sized {
                missing.push(format!("{}:{}", path.display(), index + 1));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "wrapped texts without `min-height: self.preferred-height;`:\n{}",
        missing.join("\n")
    );
}
