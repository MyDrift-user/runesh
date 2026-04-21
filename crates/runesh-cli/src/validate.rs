//! Input validation and escaping helpers for scaffolding.
//!
//! Project names and free-form user input are embedded in TOML, YAML, and
//! Markdown files. Injection into these formats can produce malformed
//! Cargo.toml, break YAML parsers, or alter Markdown structure; validating
//! project names and escaping descriptions prevents that.

/// Validate a project name against `^[a-z][a-z0-9_-]{0,63}$`.
///
/// Lowercase, starts with a letter, 1-64 chars, letters/digits/underscore/hyphen only.
pub fn valid_project_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return false;
    }
    if !(bytes[0].is_ascii_lowercase()) {
        return false;
    }
    bytes
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_' || *b == b'-')
}

/// Return a descriptive error if the name is invalid, else Ok(()).
pub fn check_project_name(name: &str) -> Result<(), String> {
    if valid_project_name(name) {
        Ok(())
    } else {
        Err(format!(
            "invalid project name '{name}': must match ^[a-z][a-z0-9_-]{{0,63}}$"
        ))
    }
}

/// Escape a string for use as a TOML basic string value.
/// Returns a double-quoted TOML string (quotes included).
pub fn toml_string(value: &str) -> String {
    // Use the toml crate to guarantee correctness instead of hand-rolling.
    toml::Value::String(value.to_string()).to_string()
}

/// Escape a string for embedding in Markdown prose: backticks, backslashes,
/// underscores, asterisks, and angle brackets are prefixed with a backslash.
/// HTML-significant characters (`<`, `>`) are replaced with their entities.
pub fn markdown_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' | '`' | '*' | '_' | '{' | '}' | '[' | ']' | '(' | ')' | '#' | '+' | '!' | '|' => {
                out.push('\\');
                out.push(ch);
            }
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            // Line terminators inside prose break paragraph structure; normalize
            // to a single space.
            '\r' | '\n' => out.push(' '),
            _ => out.push(ch),
        }
    }
    out
}

/// Escape a string for use as a YAML scalar. Emits a double-quoted YAML
/// string with backslash/quote escapes applied, safe in flow and block
/// contexts.
#[allow(dead_code)]
pub fn yaml_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_name_regex() {
        assert!(valid_project_name("foo"));
        assert!(valid_project_name("my-app-2"));
        assert!(valid_project_name("a_b-c123"));
        assert!(valid_project_name(&"a".repeat(64)));

        assert!(!valid_project_name(""));
        assert!(!valid_project_name("A"));
        assert!(!valid_project_name("1foo"));
        assert!(!valid_project_name("foo bar"));
        assert!(!valid_project_name("foo.bar"));
        assert!(!valid_project_name("foo/bar"));
        assert!(!valid_project_name(&"a".repeat(65)));
        assert!(!valid_project_name("../evil"));
    }

    #[test]
    fn toml_escapes_quotes_and_backslashes() {
        let tricky = r#"he said "hi" \ \"#;
        let out = toml_string(tricky);
        // Round-trip via toml crate.
        let doc: toml::Table = format!("k = {out}").parse().unwrap();
        assert_eq!(doc["k"].as_str().unwrap(), tricky);
    }

    #[test]
    fn toml_escapes_injection_attempt() {
        // If a user supplies a description trying to break out of the
        // string and add new keys, the output must remain a single string.
        let bad = r#"oops"
evil = true
foo = ""#;
        let out = toml_string(bad);
        let doc: toml::Table = format!("k = {out}").parse().unwrap();
        assert_eq!(doc["k"].as_str().unwrap(), bad);
        assert!(!doc.contains_key("evil"));
    }

    #[test]
    fn markdown_escapes_structure() {
        let out = markdown_escape("[*hi*](evil) `cmd`");
        assert!(out.contains("\\["));
        assert!(out.contains("\\]"));
        assert!(out.contains("\\*"));
        assert!(out.contains("\\("));
        assert!(out.contains("\\)"));
        assert!(out.contains("\\`"));
    }

    #[test]
    fn yaml_escapes_quotes_and_newlines() {
        let out = yaml_string("line1\nline2\"end");
        assert!(out.starts_with('"') && out.ends_with('"'));
        assert!(out.contains("\\n"));
        assert!(out.contains("\\\""));
    }
}
