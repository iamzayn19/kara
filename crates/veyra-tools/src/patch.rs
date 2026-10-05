//! Text edits: exact search/replace and tolerant unified-diff application.
//!
//! Local models often produce slightly imperfect diffs (wrong hunk counts,
//! missing leading space on blank context lines, shifted line numbers). The
//! applier ignores hunk counts, locates each hunk by its context, prefers the
//! position nearest the stated line number, and falls back to
//! whitespace-insensitive matching. It never applies a hunk whose context is
//! ambiguous or missing.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: usize,
    pub old: Vec<String>,
    pub new: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePatch {
    /// `None` when the file is being created (`--- /dev/null`).
    pub old_path: Option<String>,
    /// `None` when the file is being deleted (`+++ /dev/null`).
    pub new_path: Option<String>,
    pub hunks: Vec<Hunk>,
}

fn strip_prefix_path(p: &str) -> Option<String> {
    let p = p.trim();
    let p = p.split('\t').next().unwrap_or(p).trim();
    if p == "/dev/null" {
        return None;
    }
    let p = p
        .strip_prefix("a/")
        .or_else(|| p.strip_prefix("b/"))
        .unwrap_or(p);
    Some(p.trim_matches('"').to_string())
}

pub fn parse_unified(text: &str) -> Result<Vec<FilePatch>, String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut patches = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some(old) = line.strip_prefix("--- ") {
            let Some(new) = lines.get(i + 1).and_then(|l| l.strip_prefix("+++ ")) else {
                return Err(format!("line {}: `---` header without `+++`", i + 1));
            };
            let mut fp = FilePatch {
                old_path: strip_prefix_path(old),
                new_path: strip_prefix_path(new),
                hunks: Vec::new(),
            };
            i += 2;
            while i < lines.len() && !lines[i].starts_with("--- ") {
                let l = lines[i];
                if let Some(rest) = l.strip_prefix("@@") {
                    let old_start = rest
                        .trim()
                        .strip_prefix('-')
                        .and_then(|r| r.split([',', ' ']).next())
                        .and_then(|n| n.parse::<usize>().ok())
                        .unwrap_or(1);
                    let mut h = Hunk {
                        old_start,
                        old: Vec::new(),
                        new: Vec::new(),
                    };
                    i += 1;
                    while i < lines.len() {
                        let hl = lines[i];
                        if hl.starts_with("@@")
                            || hl.starts_with("--- ")
                            || hl.starts_with("diff --git")
                        {
                            break;
                        }
                        if hl.starts_with("\\ No newline") {
                            i += 1;
                            continue;
                        }
                        match hl.chars().next() {
                            Some('+') => h.new.push(hl[1..].to_string()),
                            Some('-') => h.old.push(hl[1..].to_string()),
                            Some(' ') => {
                                h.old.push(hl[1..].to_string());
                                h.new.push(hl[1..].to_string());
                            }
                            None => {
                                h.old.push(String::new());
                                h.new.push(String::new());
                            }
                            Some(_) => {
                                // Context line missing its leading space.
                                h.old.push(hl.to_string());
                                h.new.push(hl.to_string());
                            }
                        }
                        i += 1;
                    }
                    // Trailing blank context lines often come from the
                    // separator between files; drop them.
                    while h.old.last().map(|s| s.is_empty()).unwrap_or(false)
                        && h.new.last().map(|s| s.is_empty()).unwrap_or(false)
                        && h.old.len() > 1
                    {
                        h.old.pop();
                        h.new.pop();
                    }
                    fp.hunks.push(h);
                } else {
                    i += 1;
                }
            }
            if fp.old_path.is_none() && fp.new_path.is_none() {
                return Err("patch header has /dev/null on both sides".into());
            }
            patches.push(fp);
        } else {
            i += 1;
        }
    }
    if patches.is_empty() {
        return Err("no file headers (`--- a/path` / `+++ b/path`) found in patch".into());
    }
    Ok(patches)
}

#[derive(Clone, Copy)]
enum Fuzz {
    Exact,
    TrailingWs,
    AllWs,
}

fn eq(a: &str, b: &str, f: Fuzz) -> bool {
    match f {
        Fuzz::Exact => a == b,
        Fuzz::TrailingWs => a.trim_end() == b.trim_end(),
        Fuzz::AllWs => {
            a.split_whitespace().collect::<Vec<_>>() == b.split_whitespace().collect::<Vec<_>>()
        }
    }
}

fn find_block(
    hay: &[String],
    needle: &[String],
    near: usize,
    fuzz: Fuzz,
) -> Result<Option<usize>, String> {
    if needle.is_empty() {
        return Ok(Some(near.min(hay.len())));
    }
    if needle.len() > hay.len() {
        return Ok(None);
    }
    let mut hits = Vec::new();
    for start in 0..=hay.len() - needle.len() {
        if needle
            .iter()
            .enumerate()
            .all(|(k, n)| eq(&hay[start + k], n, fuzz))
        {
            hits.push(start);
        }
    }
    match hits.len() {
        0 => Ok(None),
        1 => Ok(Some(hits[0])),
        _ => {
            // Pick the hit closest to the stated position; refuse if two are
            // equally close.
            hits.sort_by_key(|h| (*h as i64 - near as i64).abs());
            let d0 = (hits[0] as i64 - near as i64).abs();
            let d1 = (hits[1] as i64 - near as i64).abs();
            if d0 == d1 {
                Err(format!(
                    "hunk context matches {} places equally well (lines {} and {}); include more context",
                    hits.len(),
                    hits[0] + 1,
                    hits[1] + 1
                ))
            } else {
                Ok(Some(hits[0]))
            }
        }
    }
}

fn split_lines(text: &str) -> (Vec<String>, bool, &'static str) {
    let crlf = text.contains("\r\n");
    let trailing_nl = text.ends_with('\n');
    let lines = text
        .lines()
        .map(|l| l.strip_suffix('\r').unwrap_or(l).to_string())
        .collect();
    (lines, trailing_nl, if crlf { "\r\n" } else { "\n" })
}

fn join_lines(lines: &[String], trailing_nl: bool, eol: &str) -> String {
    let mut s = lines.join(eol);
    if trailing_nl && !lines.is_empty() {
        s.push_str(eol);
    }
    s
}

/// Apply hunks to `original`. Fails without partial application.
pub fn apply_hunks(original: &str, hunks: &[Hunk]) -> Result<String, String> {
    let (mut lines, trailing_nl, eol) = split_lines(original);
    let trailing_nl = trailing_nl || original.is_empty();
    let mut offset: i64 = 0;
    for (n, h) in hunks.iter().enumerate() {
        let near = ((h.old_start as i64 - 1) + offset).max(0) as usize;
        let mut found = None;
        for fuzz in [Fuzz::Exact, Fuzz::TrailingWs, Fuzz::AllWs] {
            if let Some(pos) = find_block(&lines, &h.old, near, fuzz)
                .map_err(|e| format!("hunk {}: {e}", n + 1))?
            {
                found = Some(pos);
                break;
            }
        }
        let Some(pos) = found else {
            let preview: Vec<&str> = h.old.iter().take(3).map(String::as_str).collect();
            return Err(format!(
                "hunk {} does not apply: context not found in file (first lines: {:?}). Re-read the file and retry.",
                n + 1,
                preview
            ));
        };
        lines.splice(pos..pos + h.old.len(), h.new.iter().cloned());
        offset += h.new.len() as i64 - h.old.len() as i64;
    }
    Ok(join_lines(&lines, trailing_nl, eol))
}

/// Exact search/replace. With `replace_all = false`, `old` must occur
/// exactly once. Falls back to a whitespace-insensitive line match when the
/// exact text is absent and the fuzzy match is unique.
pub fn edit_replace(
    original: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<(String, usize), String> {
    if old.is_empty() {
        return Err(
            "old_string must not be empty (use write_file/create_file to create content)".into(),
        );
    }
    if old == new {
        return Err("old_string and new_string are identical; nothing to change".into());
    }
    // Normalise line endings of the search text to the file's.
    let crlf = original.contains("\r\n");
    let fix = |s: &str| {
        if crlf {
            s.replace("\r\n", "\n").replace('\n', "\r\n")
        } else {
            s.replace("\r\n", "\n")
        }
    };
    let (old_n, new_n) = (fix(old), fix(new));
    let count = original.matches(&old_n).count();
    match count {
        1 => return Ok((original.replacen(&old_n, &new_n, 1), 1)),
        n if n > 1 && replace_all => return Ok((original.replace(&old_n, &new_n), n)),
        n if n > 1 => {
            return Err(format!(
                "old_string occurs {n} times; add surrounding lines to make it unique or set replace_all"
            ))
        }
        _ => {}
    }

    // Whitespace-tolerant fallback on whole lines.
    let (lines, trailing_nl, eol) = split_lines(original);
    let old_lines: Vec<String> = old.lines().map(str::to_string).collect();
    let new_lines: Vec<String> = new.lines().map(str::to_string).collect();
    for fuzz in [Fuzz::TrailingWs, Fuzz::AllWs] {
        let mut hits = Vec::new();
        if old_lines.len() <= lines.len() && !old_lines.is_empty() {
            for s in 0..=lines.len() - old_lines.len() {
                if old_lines
                    .iter()
                    .enumerate()
                    .all(|(k, o)| eq(&lines[s + k], o, fuzz))
                {
                    hits.push(s);
                }
            }
        }
        if hits.len() == 1 {
            let s = hits[0];
            // Re-indent new lines by the indentation delta of the first line.
            let actual_indent = leading_ws(&lines[s]);
            let given_indent = leading_ws(&old_lines[0]);
            let adjusted: Vec<String> = new_lines
                .iter()
                .map(|l| {
                    if l.trim().is_empty() {
                        String::new()
                    } else if let Some(rest) = l.strip_prefix(given_indent) {
                        format!("{actual_indent}{rest}")
                    } else {
                        l.clone()
                    }
                })
                .collect();
            let mut out = lines.clone();
            out.splice(s..s + old_lines.len(), adjusted);
            return Ok((join_lines(&out, trailing_nl, eol), 1));
        }
        if hits.len() > 1 {
            return Err(format!(
                "old_string matches {} places after ignoring whitespace; include more context",
                hits.len()
            ));
        }
    }
    Err(closest_hint(&lines, &old_lines))
}

fn leading_ws(s: &str) -> &str {
    &s[..s.len() - s.trim_start().len()]
}

fn closest_hint(lines: &[String], old_lines: &[String]) -> String {
    let first = old_lines.iter().find(|l| !l.trim().is_empty());
    let mut msg = String::from(
        "old_string not found in file. To add new code (rather than change existing code), use insert_lines with a line number from read_file.",
    );
    if let Some(first) = first {
        let target = first.trim();
        let best = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| !l.trim().is_empty())
            .map(|(i, l)| (i, similar::TextDiff::from_chars(l.trim(), target).ratio()))
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        if let Some((i, ratio)) = best {
            if ratio > 0.5 {
                msg.push_str(&format!(
                    " Closest match is line {}: `{}`. Re-read that range and copy the text exactly.",
                    i + 1,
                    lines[i].trim()
                ));
            }
        }
    }
    msg
}

/// Unified diff between two texts for previews and change events.
pub fn unified_diff(path: &str, before: &str, after: &str) -> String {
    similar::TextDiff::from_lines(before, after)
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{path}"), &format!("b/{path}"))
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "def total(items):\n    s = 0\n    for i in items:\n        s += i.price\n    return s\n\ndef discount(total):\n    return total * 0.9\n";

    #[test]
    fn exact_replace() {
        let (out, n) =
            edit_replace(FILE, "return total * 0.9", "return total * 0.8", false).unwrap();
        assert_eq!(n, 1);
        assert!(out.contains("0.8") && !out.contains("0.9"));
    }

    #[test]
    fn ambiguous_replace_is_refused() {
        let src = "a = 1\na = 1\n";
        assert!(edit_replace(src, "a = 1", "a = 2", false)
            .unwrap_err()
            .contains("2 times"));
        assert_eq!(
            edit_replace(src, "a = 1", "a = 2", true).unwrap().0,
            "a = 2\na = 2\n"
        );
    }

    #[test]
    fn whitespace_tolerant_replace_reindents() {
        let (out, _) = edit_replace(
            FILE,
            "for i in items:\n    s += i.price",
            "for i in items:\n    s += i.price * i.qty",
            false,
        )
        .unwrap();
        assert!(
            out.contains("    for i in items:\n        s += i.price * i.qty\n"),
            "{out}"
        );
    }

    #[test]
    fn not_found_gives_hint() {
        let err = edit_replace(FILE, "return total * 0.95", "x", false).unwrap_err();
        assert!(err.contains("line 8"), "{err}");
    }

    #[test]
    fn crlf_is_preserved() {
        let src = "a\r\nb\r\nc\r\n";
        let (out, _) = edit_replace(src, "b\nc", "B\nC", false).unwrap();
        assert_eq!(out, "a\r\nB\r\nC\r\n");
    }

    #[test]
    fn parse_and_apply_unified() {
        let patch = "--- a/cart.py\n+++ b/cart.py\n@@ -7,2 +7,2 @@\n def discount(total):\n-    return total * 0.9\n+    return total * 0.8\n";
        let fps = parse_unified(patch).unwrap();
        assert_eq!(fps.len(), 1);
        assert_eq!(fps[0].old_path.as_deref(), Some("cart.py"));
        let out = apply_hunks(FILE, &fps[0].hunks).unwrap();
        assert!(out.ends_with("return total * 0.8\n"));
    }

    #[test]
    fn tolerates_wrong_line_numbers_and_counts() {
        let patch = "--- a/cart.py\n+++ b/cart.py\n@@ -1,99 +1,99 @@\n def discount(total):\n-    return total * 0.9\n+    return total * 0.8\n";
        let fps = parse_unified(patch).unwrap();
        assert!(apply_hunks(FILE, &fps[0].hunks).unwrap().contains("0.8"));
    }

    #[test]
    fn tolerates_blank_context_without_space() {
        let patch = "--- a/cart.py\n+++ b/cart.py\n@@ -5,4 +5,4 @@\n     return s\n\n-def discount(total):\n+def discount(total, rate=0.9):\n";
        let fps = parse_unified(patch).unwrap();
        let out = apply_hunks(FILE, &fps[0].hunks).unwrap();
        assert!(out.contains("def discount(total, rate=0.9):"));
    }

    #[test]
    fn missing_context_fails_cleanly() {
        let patch =
            "--- a/cart.py\n+++ b/cart.py\n@@ -1,1 +1,1 @@\n-def nothing_here():\n+def x():\n";
        let fps = parse_unified(patch).unwrap();
        assert!(apply_hunks(FILE, &fps[0].hunks)
            .unwrap_err()
            .contains("does not apply"));
    }

    #[test]
    fn create_delete_and_multi_file() {
        let patch = "--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,2 @@\n+hello\n+world\n--- a/old.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-bye\n";
        let fps = parse_unified(patch).unwrap();
        assert_eq!(fps.len(), 2);
        assert_eq!(fps[0].old_path, None);
        assert_eq!(apply_hunks("", &fps[0].hunks).unwrap(), "hello\nworld\n");
        assert_eq!(fps[1].new_path, None);
        assert!(parse_unified("just text").is_err());
    }

    #[test]
    fn diff_output() {
        let d = unified_diff("x.txt", "a\nb\n", "a\nc\n");
        assert!(d.contains("--- a/x.txt") && d.contains("-b") && d.contains("+c"));
    }
}
