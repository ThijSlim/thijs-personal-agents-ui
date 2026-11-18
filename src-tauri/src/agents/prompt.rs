//! Pure helpers for an agent's conversation: cleaning terminal output,
//! spotting a permission prompt, and the keys/params used to answer one.

use serde::Serialize;
use serde_json::{Value, json};

/// Keys that pick "Yes" on a Claude Code permission prompt (verified live:
/// `1` runs the tool and the agent goes back to work).
pub const APPROVE_KEYS: &[&str] = &["1"];
/// Keys that refuse a Claude Code permission prompt (verified live: `esc`
/// cancels the tool call, the agent is interrupted and waits for input).
pub const DENY_KEYS: &[&str] = &["esc"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionPrompt {
    pub question: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub text: String,
    pub permission: Option<PermissionPrompt>,
    pub ended: bool,
}

/// Make terminal output readable as plain text: drop ANSI escape sequences
/// and control characters (keeping `\n` and `\t`), trailing spaces on each
/// line, blank lines at the end, and blank-line runs longer than two.
pub fn clean_output(raw: &str) -> String {
    let stripped = strip_escapes(raw);
    let mut out: Vec<&str> = Vec::new();
    let mut blanks = 0;
    for line in stripped.split('\n') {
        let line = line.trim_end();
        if line.is_empty() {
            blanks += 1;
            if blanks > 2 {
                continue;
            }
        } else {
            blanks = 0;
        }
        out.push(line);
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    while out.first().is_some_and(|l| l.is_empty()) {
        out.remove(0);
    }
    out.join("\n")
}

/// Remove CSI (`ESC [ ... final`), OSC/DCS-style strings (`ESC ] ... BEL|ST`),
/// other `ESC`-introduced sequences, and C0/C1 controls except `\n`/`\t`.
fn strip_escapes(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.next() {
                Some('[') => {
                    // Parameters/intermediates until a final byte in @..~.
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']' | 'P' | 'X' | '^' | '_') => {
                    // String until BEL or ST (ESC \\).
                    while let Some(c) = chars.next() {
                        if c == '\x07' {
                            break;
                        }
                        if c == '\x1b' {
                            if chars.peek() == Some(&'\\') {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
                Some(c) if (' '..='/').contains(&c) => {
                    // nF sequence: intermediates then one final byte.
                    for c in chars.by_ref() {
                        if !(' '..='/').contains(&c) {
                            break;
                        }
                    }
                }
                _ => {} // two-character sequence (or a lone trailing ESC)
            },
            '\n' | '\t' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Find a Claude Code style permission prompt ("Do you want to ...?"
/// followed by numbered options) on the screen and return its question:
/// the prompt box from its top rule down to the question line, without
/// tips and dashed separators.
pub fn detect_permission(screen: &str) -> Option<PermissionPrompt> {
    let lines: Vec<&str> = screen.lines().map(str::trim).collect();
    let ask = lines.iter().rposition(|l| l.starts_with("Do you want to") && l.ends_with('?'))?;
    let has_options = lines[ask + 1..]
        .iter()
        .filter(|l| !l.is_empty())
        .take(2)
        .any(|l| l.trim_start_matches('❯').trim_start().starts_with("1. "));
    if !has_options {
        return None;
    }
    let start = lines[..ask].iter().rposition(|l| is_rule(l, '─')).map_or(ask, |i| i + 1);
    let question: Vec<&str> = lines[start..=ask]
        .iter()
        .copied()
        .filter(|l| !l.is_empty() && !is_rule(l, '╌') && !l.starts_with("Tip:"))
        .collect();
    Some(PermissionPrompt { question: question.join("\n") })
}

fn is_rule(line: &str, ch: char) -> bool {
    line.chars().count() >= 3 && line.chars().all(|c| c == ch)
}

/// `agent.prompt` params for a reply. The text is passed through exactly as
/// typed; only empty or whitespace-only text is rejected.
pub fn reply_params(pane_id: &str, text: &str) -> Result<Value, String> {
    if text.trim().is_empty() {
        return Err("reply is empty".into());
    }
    Ok(json!({ "target": pane_id, "text": text }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERMISSION: &str = include_str!("../../tests/fixtures/claude_permission.txt");
    const IDLE: &str = include_str!("../../tests/fixtures/claude_idle.txt");

    #[test]
    fn clean_output_strips_ansi_and_trailing_blanks() {
        let raw = "\x1b[1;32mgreen\x1b[0m text\r\n\x1b]0;title\x07next\x08 line  \n\n\n\n\n\nafter\x1b(B\n \n\n";
        assert_eq!(clean_output(raw), "green text\nnext line\n\n\nafter");
    }

    #[test]
    fn clean_output_keeps_unicode_and_short_blank_runs() {
        assert_eq!(clean_output("❯ héllo 日本\n\n\nok\n"), "❯ héllo 日本\n\n\nok");
        assert_eq!(clean_output(""), "");
        assert_eq!(clean_output("\n\n  \n"), "");
    }

    #[test]
    fn detects_permission_in_fixture() {
        let p = detect_permission(PERMISSION).expect("permission prompt");
        assert!(!p.question.trim().is_empty());
        assert!(p.question.contains("Bash command"), "{:?}", p.question);
        assert!(p.question.contains("touch probe.txt"), "{:?}", p.question);
        assert!(p.question.contains("Do you want to proceed?"), "{:?}", p.question);
        assert!(!p.question.contains("Tip:"), "{:?}", p.question);
        assert!(!p.question.contains('╌'), "{:?}", p.question);
        // The conversation above the prompt is not part of the question.
        assert!(!p.question.contains("Run the shell command"), "{:?}", p.question);
    }

    #[test]
    fn no_permission_when_idle() {
        assert_eq!(detect_permission(IDLE), None);
        assert_eq!(detect_permission(""), None);
        // A question without numbered options is just conversation.
        assert_eq!(detect_permission("Do you want to proceed?\nsure"), None);
    }

    #[test]
    fn reply_params_carry_text_byte_for_byte() {
        let text = "line1\nline2 — ✓ 日本\n  indented\t tab ";
        let v = reply_params("w1:p1", text).unwrap();
        assert_eq!(v, json!({ "target": "w1:p1", "text": text }));
        assert_eq!(v["text"].as_str().unwrap().as_bytes(), text.as_bytes());
    }

    #[test]
    fn reply_params_reject_empty_and_whitespace() {
        for t in ["", " ", "\n\t \n"] {
            assert!(reply_params("w1:p1", t).is_err(), "{t:?}");
        }
    }

    #[test]
    fn keys_are_the_verified_ones() {
        assert_eq!(APPROVE_KEYS, ["1"]);
        assert_eq!(DENY_KEYS, ["esc"]);
    }
}
