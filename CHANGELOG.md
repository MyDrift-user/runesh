# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- `runesh update` self-update command (pulls latest release from GitHub).
- CI workflow (`cargo fmt`, `clippy`, `check`, `test`).
- Release workflow building Windows (`.msi` + `.zip`), macOS (`.tar.gz`), and Linux (`.tar.gz`) artifacts on tag push.
- `install.ps1` and `install.sh` one-liner installers.
- `rust-toolchain.toml` pinning the stable channel.

## [0.1.0]

Initial workspace with `runesh-cli`, `runesh-core`, `runesh-auth`, `runesh-inventory`, `runesh-remote`, `runesh-desktop`, `runesh-vfs`, `runesh-tun`, `runesh-tauri`, and `@runesh/ui`.
