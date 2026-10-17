//! Source checks for the rules in ui/theme.slint and the localization policy: colours
//! and durations come only from the Theme global, and user-visible text goes through
//! `@tr()`.

use std::fs;
use std::path::{Path, PathBuf};

/// String literals that are not translatable text: the product name, the font family,
/// a URL, and the empty string (used in comparisons).
const ALLOWED_LITERALS: &[&str] = &["\"VCMP\"", "\"Sora\"", "\"https://slint.dev\"", "\"\""];

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

/// Removes every `@tr(...)` call, including plural forms such as
/// `@tr("{n} track" | "{n} tracks" % count)`, leaving the rest of the line.
fn without_tr_calls(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(start) = rest.find("@tr(") {
        out.push_str(&rest[..start]);
        let call = &rest[start + 4..];
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

#[test]
fn text_is_translatable() {
    let mut violations = Vec::new();
    for path in slint_sources() {
        for (number, line) in code_lines(&path) {
            // Path drawing commands are data, not text.
            if line.starts_with("commands:") {
                continue;
            }
            let mut remaining = without_tr_calls(&line);
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
