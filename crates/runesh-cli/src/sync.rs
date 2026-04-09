//! `runesh sync` command.
//!
//! Fetches the shared CLAUDE.base.md template from the RUNESH repo and
//! updates the current project's CLAUDE.md. The shared section (everything
//! between the title line and the `<!-- PROJECT -->` marker) is replaced,
//! while all project-specific content below the marker is preserved.

use std::fs;
use std::path::PathBuf;

const RAW_BASE_URL: &str =
    "https://raw.githubusercontent.com/mydrift-user/runesh/main/templates/CLAUDE.base.md";

const MARKER: &str = "<!-- PROJECT -->";

pub fn run() -> Result<(), String> {
    let claude_path = find_claude_md()?;

    let existing = fs::read_to_string(&claude_path)
        .map_err(|e| format!("Failed to read {}: {e}", claude_path.display()))?;

    let base = fetch_base_template()?;

    let updated = merge(&existing, &base);

    fs::write(&claude_path, &updated)
        .map_err(|e| format!("Failed to write {}: {e}", claude_path.display()))?;

    println!(
        "\x1b[32mupdated\x1b[0m {}",
        claude_path.display()
    );
    Ok(())
}

fn find_claude_md() -> Result<PathBuf, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("Cannot get cwd: {e}"))?;
    let path = cwd.join("CLAUDE.md");
    if path.exists() {
        Ok(path)
    } else {
        Err("No CLAUDE.md found in the current directory. Run this from your project root.".into())
    }
}

fn fetch_base_template() -> Result<String, String> {
    println!("Fetching shared rules from RUNESH...");

    let output = std::process::Command::new("curl")
        .args(["-sfL", RAW_BASE_URL])
        .output()
        .map_err(|e| format!("Failed to run curl: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "Failed to fetch template from {RAW_BASE_URL} (status {})",
            output.status
        ));
    }

    String::from_utf8(output.stdout).map_err(|e| format!("Invalid UTF-8 in template: {e}"))
}

fn merge(existing: &str, base: &str) -> String {
    // If the file has a PROJECT marker, keep everything after it.
    if let Some(marker_pos) = existing.find(MARKER) {
        let project_section = &existing[marker_pos + MARKER.len()..];
        let title_line = existing.lines().next().unwrap_or("# Project");

        format!("{title_line}\n\n{base}\n{MARKER}\n{project_section}")
    } else {
        // No marker yet. Find the first `## ` heading that is NOT in the base
        // template (i.e. the start of project-specific content).
        // Strategy: take the title line, insert base + marker, then append
        // everything after the title.
        let title_line = existing.lines().next().unwrap_or("# Project");
        let rest = existing
            .strip_prefix(title_line)
            .unwrap_or("")
            .trim_start_matches('\n')
            .trim_start_matches('\r');

        format!("{title_line}\n\n{base}\n{MARKER}\n\n{rest}\n")
    }
}
