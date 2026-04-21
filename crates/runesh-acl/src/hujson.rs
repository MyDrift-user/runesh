//! HuJSON parser: JSON with comments and trailing commas.
//!
//! HuJSON (Human JSON) extends JSON with:
//! - Line comments: `// comment`
//! - Block comments: `/* comment */`
//! - Trailing commas in arrays and objects
//!
//! This module strips these extensions to produce valid JSON in a single
//! pass over the input, then delegates to serde_json for parsing.

use crate::AclError;

/// Internal tokenizer state.
#[derive(Clone, Copy)]
enum State {
    Normal,
    InString,
    InStringEscape,
    InLineComment,
    InBlockComment,
    InBlockCommentStar,
}

/// Strip HuJSON extensions from input, producing valid JSON.
///
/// Handles:
/// - `// line comments` (removed with the newline)
/// - `/* block comments */` (removed entirely)
/// - Trailing commas before `]` or `}` (removed)
/// - Preserves strings containing `//` or `/*` or escaped quotes literally
///
/// This is a single-pass tokenizer that tracks string state once. Trailing
/// commas are buffered: when a comma is encountered outside a string, it is
/// deferred. If the next non-whitespace character is `]` or `}` the comma is
/// discarded; otherwise it is emitted.
pub fn to_json(input: &str) -> Result<String, AclError> {
    let mut out = String::with_capacity(input.len());
    let mut state = State::Normal;
    // A comma waiting to be emitted; we hold trailing whitespace alongside it
    // so we can strip the comma while still preserving newlines etc.
    let mut pending_comma: Option<String> = None;

    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match state {
            State::Normal => {
                // If we have a pending comma, decide what to do with it now.
                if let Some(buf) = pending_comma.as_mut() {
                    if c.is_whitespace() {
                        buf.push(c);
                        continue;
                    }
                    // Comments must not flush the pending comma: they are
                    // invisible in the final JSON, so a trailing comma
                    // followed by a comment then `}` or `]` is still
                    // a trailing comma and must be dropped.
                    if c == '/' {
                        match chars.peek() {
                            Some('/') => {
                                chars.next();
                                state = State::InLineComment;
                                continue;
                            }
                            Some('*') => {
                                chars.next();
                                state = State::InBlockComment;
                                continue;
                            }
                            _ => {}
                        }
                    }
                    if c == ']' || c == '}' {
                        // Trailing comma: drop the comma, keep whitespace.
                        let ws: String = buf.chars().skip(1).collect();
                        out.push_str(&ws);
                        pending_comma = None;
                        out.push(c);
                        continue;
                    }
                    // Not a trailing comma: flush the buffer unchanged.
                    out.push_str(buf);
                    pending_comma = None;
                }

                match c {
                    '"' => {
                        out.push(c);
                        state = State::InString;
                    }
                    '/' => match chars.peek() {
                        Some('/') => {
                            chars.next();
                            state = State::InLineComment;
                        }
                        Some('*') => {
                            chars.next();
                            state = State::InBlockComment;
                        }
                        _ => out.push(c),
                    },
                    ',' => {
                        // Defer the comma until we know what follows.
                        pending_comma = Some(String::from(','));
                    }
                    _ => out.push(c),
                }
            }
            State::InString => {
                out.push(c);
                match c {
                    '\\' => state = State::InStringEscape,
                    '"' => state = State::Normal,
                    _ => {}
                }
            }
            State::InStringEscape => {
                out.push(c);
                state = State::InString;
            }
            State::InLineComment => {
                if c == '\n' {
                    out.push('\n');
                    state = State::Normal;
                }
            }
            State::InBlockComment => {
                if c == '*' {
                    state = State::InBlockCommentStar;
                } else if c == '\n' {
                    out.push('\n');
                }
            }
            State::InBlockCommentStar => {
                if c == '/' {
                    state = State::Normal;
                } else if c == '*' {
                    // stay in star state
                } else {
                    if c == '\n' {
                        out.push('\n');
                    }
                    state = State::InBlockComment;
                }
            }
        }
    }

    // Flush any pending comma at EOF (it was not a trailing comma, so keep it).
    if let Some(buf) = pending_comma {
        out.push_str(&buf);
    }

    match state {
        State::Normal => Ok(out),
        State::InString | State::InStringEscape => {
            Err(AclError::InvalidHuJson("unterminated string".into()))
        }
        State::InBlockComment | State::InBlockCommentStar => {
            Err(AclError::InvalidHuJson("unterminated block comment".into()))
        }
        // An unterminated line comment just means the file ended without a newline,
        // which is perfectly fine.
        State::InLineComment => Ok(out),
    }
}

/// Parse a HuJSON string into a serde_json::Value.
pub fn parse(input: &str) -> Result<serde_json::Value, AclError> {
    let json = to_json(input)?;
    serde_json::from_str(&json).map_err(|e| AclError::InvalidHuJson(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_line_comments() {
        let input = r#"{
            // this is a comment
            "key": "value" // inline comment
        }"#;
        let result: serde_json::Value = parse(input).unwrap();
        assert_eq!(result["key"], "value");
    }

    #[test]
    fn strip_block_comments() {
        let input = r#"{
            /* block comment */
            "key": /* inline */ "value"
        }"#;
        let result: serde_json::Value = parse(input).unwrap();
        assert_eq!(result["key"], "value");
    }

    #[test]
    fn trailing_commas() {
        let input = r#"{
            "a": [1, 2, 3,],
            "b": {"x": 1,},
        }"#;
        let result: serde_json::Value = parse(input).unwrap();
        assert_eq!(result["a"][2], 3);
        assert_eq!(result["b"]["x"], 1);
    }

    #[test]
    fn preserve_comments_in_strings() {
        let input = r#"{"url": "https://example.com/path"}"#;
        let result: serde_json::Value = parse(input).unwrap();
        assert_eq!(result["url"], "https://example.com/path");
    }

    #[test]
    fn slash_in_string_not_comment() {
        let input = r#"{"msg": "use // for comments"}"#;
        let result: serde_json::Value = parse(input).unwrap();
        assert_eq!(result["msg"], "use // for comments");
    }

    #[test]
    fn unterminated_block_comment() {
        let input = r#"{"key": /* never closed "value"}"#;
        assert!(parse(input).is_err());
    }

    #[test]
    fn combined() {
        let input = r#"{
            // Tailscale-style ACL
            "groups": {
                "group:admin": ["user@example.com",], /* trailing comma */
            },
            "acls": [
                {
                    "action": "accept",
                    "src": ["group:admin"],
                    "dst": ["*:*"],
                },
            ],
        }"#;
        let result: serde_json::Value = parse(input).unwrap();
        assert_eq!(result["groups"]["group:admin"][0], "user@example.com");
        assert_eq!(result["acls"][0]["action"], "accept");
    }

    #[test]
    fn escaped_quotes_adjacent_to_commas() {
        // Pathological: escaped quote immediately before comma before bracket.
        let input = r#"{"a": ["he said \"hi,\"", "bye",]}"#;
        let result: serde_json::Value = parse(input).unwrap();
        assert_eq!(result["a"][0], "he said \"hi,\"");
        assert_eq!(result["a"][1], "bye");
    }

    #[test]
    fn escaped_backslash_then_quote() {
        // String containing a backslash and then ending; trailing comma after.
        let input = r#"{"p": "c:\\path\\", "q": 1,}"#;
        let result: serde_json::Value = parse(input).unwrap();
        assert_eq!(result["p"], "c:\\path\\");
        assert_eq!(result["q"], 1);
    }

    #[test]
    fn comment_markers_inside_string() {
        // // and /* inside strings must not be mistaken for comments.
        let input = r#"{"a": "// not a comment", "b": "/* also not */"}"#;
        let result: serde_json::Value = parse(input).unwrap();
        assert_eq!(result["a"], "// not a comment");
        assert_eq!(result["b"], "/* also not */");
    }

    #[test]
    fn nested_block_comment_stars() {
        // Multiple stars before the closing slash.
        let input = r#"{"a": 1 /*** hi ***/}"#;
        let result: serde_json::Value = parse(input).unwrap();
        assert_eq!(result["a"], 1);
    }

    #[test]
    fn trailing_comma_after_string_ending_in_quote() {
        let input = r#"{"a": "x",}"#;
        let result: serde_json::Value = parse(input).unwrap();
        assert_eq!(result["a"], "x");
    }

    #[test]
    fn comma_between_values_preserved() {
        let input = r#"{"a": 1, "b": 2}"#;
        let json = to_json(input).unwrap();
        assert_eq!(json, r#"{"a": 1, "b": 2}"#);
    }

    #[test]
    fn unterminated_string_rejected() {
        let input = r#"{"a": "unterminated"#;
        assert!(parse(input).is_err());
    }
}
