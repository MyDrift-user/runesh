//! HuJSON parser: JSON with comments and trailing commas.
//!
//! HuJSON (Human JSON) extends JSON with:
//! - Line comments: `// comment`
//! - Block comments: `/* comment */`
//! - Trailing commas in arrays and objects
//!
//! This module strips these extensions to produce valid JSON,
//! then delegates to serde_json for parsing.

use crate::AclError;

/// Strip HuJSON extensions from input, producing valid JSON.
///
/// Handles:
/// - `// line comments` (removed with the newline)
/// - `/* block comments */` (removed entirely)
/// - Trailing commas before `]` or `}` (removed)
/// - Preserves strings containing `//` or `/*` literally
pub fn to_json(input: &str) -> Result<String, AclError> {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escape_next = false;

    while let Some(c) = chars.next() {
        if escape_next {
            out.push(c);
            escape_next = false;
            continue;
        }

        if in_string {
            out.push(c);
            if c == '\\' {
                escape_next = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }

        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '/' => {
                match chars.peek() {
                    Some('/') => {
                        // Line comment: skip until newline
                        chars.next();
                        for nc in chars.by_ref() {
                            if nc == '\n' {
                                out.push('\n');
                                break;
                            }
                        }
                    }
                    Some('*') => {
                        // Block comment: skip until */
                        chars.next();
                        let mut found_end = false;
                        while let Some(nc) = chars.next() {
                            if nc == '*' && chars.peek() == Some(&'/') {
                                chars.next();
                                found_end = true;
                                break;
                            }
                            // Preserve newlines for line counting
                            if nc == '\n' {
                                out.push('\n');
                            }
                        }
                        if !found_end {
                            return Err(AclError::InvalidHuJson(
                                "unterminated block comment".into(),
                            ));
                        }
                    }
                    _ => {
                        out.push(c);
                    }
                }
            }
            _ => {
                out.push(c);
            }
        }
    }

    if in_string {
        return Err(AclError::InvalidHuJson("unterminated string".into()));
    }

    // Remove trailing commas: ,\s*] or ,\s*}
    let result = remove_trailing_commas(&out);

    Ok(result)
}

/// Remove trailing commas before `]` or `}`.
fn remove_trailing_commas(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut in_string = false;
    let mut escape_next = false;

    while i < len {
        let c = chars[i];

        if escape_next {
            out.push(c);
            escape_next = false;
            i += 1;
            continue;
        }

        if in_string {
            out.push(c);
            if c == '\\' {
                escape_next = true;
            } else if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        if c == '"' {
            in_string = true;
            out.push(c);
            i += 1;
            continue;
        }

        if c == ',' {
            // Look ahead: skip whitespace/newlines, check if next non-ws is ] or }
            let mut j = i + 1;
            while j < len && chars[j].is_whitespace() {
                j += 1;
            }
            if j < len && (chars[j] == ']' || chars[j] == '}') {
                // Trailing comma: skip it, but preserve whitespace
                i += 1;
                continue;
            }
        }

        out.push(c);
        i += 1;
    }

    out
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
}
