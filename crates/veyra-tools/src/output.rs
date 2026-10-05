//! Output shaping: keep what the model needs (errors, summaries, the end of a
//! log) and drop the rest, then redact credentials.

use regex::Regex;
use std::sync::OnceLock;

fn important() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)(error|fail|failed|failure|panic|exception|traceback|assert|expected|undefined|cannot|not found|denied|warning:|fatal|segmentation|\bE\d{3,4}\b|^\s*-->|^\s*at\s|:\d+:\d*)",
        )
        .expect("valid regex")
    })
}

/// Truncate `text` to roughly `max_chars`, preserving the head, the tail and
/// error-looking lines from the middle. Lines are never split mid-way except
/// when a single line exceeds the budget.
pub fn truncate_smart(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= 3 {
        return clip_chars(text, max_chars);
    }
    let head_budget = max_chars / 5;
    let tail_budget = max_chars * 2 / 5;
    let mid_budget = max_chars - head_budget - tail_budget;

    let mut head_end = 0;
    let mut used = 0;
    while head_end < lines.len() && used + lines[head_end].len() + 1 <= head_budget {
        used += lines[head_end].len() + 1;
        head_end += 1;
    }
    let mut tail_start = lines.len();
    used = 0;
    while tail_start > head_end && used + lines[tail_start - 1].len() + 1 <= tail_budget {
        used += lines[tail_start - 1].len() + 1;
        tail_start -= 1;
    }

    let mut keep = vec![false; lines.len()];
    for k in keep.iter_mut().take(head_end) {
        *k = true;
    }
    for k in keep.iter_mut().skip(tail_start) {
        *k = true;
    }
    used = 0;
    let re = important();
    for i in head_end..tail_start {
        if re.is_match(lines[i]) {
            // Include one line of context after an error line.
            for j in [i, i + 1] {
                if j < tail_start && !keep[j] {
                    let cost = lines[j].len().min(400) + 1;
                    if used + cost > mid_budget {
                        break;
                    }
                    keep[j] = true;
                    used += cost;
                }
            }
        }
        if used >= mid_budget {
            break;
        }
    }

    let mut out = String::with_capacity(max_chars + 256);
    let mut skipped = 0usize;
    for (i, line) in lines.iter().enumerate() {
        if keep[i] {
            if skipped > 0 {
                out.push_str(&format!("… [{skipped} lines omitted] …\n"));
                skipped = 0;
            }
            out.push_str(&clip_chars(line, 400));
            out.push('\n');
        } else {
            skipped += 1;
        }
    }
    if skipped > 0 {
        out.push_str(&format!("… [{skipped} lines omitted] …\n"));
    }
    out
}

pub fn clip_chars(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… [{} more bytes]", &s[..end], s.len() - end)
}

/// Strip ANSI escape sequences (colour codes from test runners).
pub fn strip_ansi(s: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07]*\x07").expect("valid regex"));
    re.replace_all(s, "").into_owned()
}

/// Final shaping for anything sent to the model: strip ANSI, truncate, redact.
pub fn for_model(text: &str, max_chars: usize) -> String {
    let clean = strip_ansi(text);
    let truncated = truncate_smart(&clean, max_chars);
    let (redacted, _) = veyra_sandbox::secrets::redact(&truncated);
    redacted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_unchanged() {
        assert_eq!(truncate_smart("a\nb\n", 100), "a\nb\n");
    }

    #[test]
    fn keeps_errors_from_the_middle() {
        let mut log = String::new();
        for i in 0..2000 {
            log.push_str(&format!("compiling crate number {i}\n"));
        }
        log.insert_str(log.len() / 2, "error[E0308]: mismatched types at src/lib.rs:10:5\n");
        let out = truncate_smart(&log, 2000);
        assert!(out.len() < 2600, "{}", out.len());
        assert!(out.contains("error[E0308]"));
        assert!(out.contains("compiling crate number 0"));
        assert!(out.contains("compiling crate number 1999"));
        assert!(out.contains("lines omitted"));
    }

    #[test]
    fn huge_single_line_is_clipped() {
        let s = "x".repeat(100_000);
        assert!(truncate_smart(&s, 1000).len() < 1100);
    }

    #[test]
    fn ansi_and_secrets_are_removed() {
        let out = for_model("\x1b[31mFAIL\x1b[0m token=\"ghp_abcdefghijklmnopqrstuvwxyz0123456789\"", 1000);
        assert!(out.starts_with("FAIL"));
        assert!(!out.contains("ghp_"));
    }
}
