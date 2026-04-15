//! `runesh update` — self-update from GitHub releases.

use self_update::cargo_crate_version;

const REPO_OWNER: &str = "MyDrift-user";
const REPO_NAME: &str = "runesh";
const BIN_NAME: &str = "runesh";

pub fn run(check_only: bool, allow_prerelease: bool) -> Result<(), Box<dyn std::error::Error>> {
    let current = cargo_crate_version!();
    println!("current version: v{current}");

    // Our release archives are packaged with a top-level directory:
    //   runesh-<target>/runesh[.exe]
    // Tell self_update where to find the binary inside that subdir.
    let bin_in_archive = if cfg!(windows) {
        "runesh-{{ target }}/{{ bin }}.exe"
    } else {
        "runesh-{{ target }}/{{ bin }}"
    };

    let updater = self_update::backends::github::Update::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .bin_name(BIN_NAME)
        .bin_path_in_archive(bin_in_archive)
        .show_download_progress(true)
        .show_output(false)
        .current_version(current)
        .no_confirm(true)
        .build()?;

    let _ = allow_prerelease; // self_update github backend ignores prereleases by default

    let latest = updater.get_latest_release()?;
    println!("latest available: {}", latest.version);

    if !allow_prerelease && latest.version.contains('-') {
        println!("latest is a prerelease; pass --prerelease to install");
        return Ok(());
    }

    if self_update::version::bump_is_greater(current, &latest.version)? {
        if check_only {
            println!("update available: v{current} -> v{}", latest.version);
            return Ok(());
        }
        println!("updating to v{}...", latest.version);
        let status = updater.update()?;
        println!("updated to v{}", status.version());
    } else {
        println!("already up to date");
    }
    Ok(())
}
