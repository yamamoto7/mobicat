# Changelog

All notable changes are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.2] - 2026-11-10

### Added

- `cargo binstall mobicat` installs the prebuilt binary from GitHub Releases.
- pidcat's flags: `-l`/`--level` (alias `--min-level`), `-t`/`--tag`, `-i`/`--ignore-tag`, and `-s`/`--serial` for `--device` (#10).

### Changed

- When the app is not installed, the error suggests installed apps with a similar ID (#8).
- When there is nothing to read from, the error says what to do: install platform-tools, start an emulator or connect a device, allow USB debugging on an unauthorized device, boot a simulator, or select Xcode (#9).
- `mobicat` with no app ID gives the same help when no device is ready, instead of "no apps found".

## [0.1.1] - 2026-11-09

### Fixed

- The README's demo image and links on crates.io.
- `cargo install --locked mobicat` in the install instructions (published to crates.io).

## [0.1.0] - 2026-11-09

### Added

- Android (`adb logcat`) and iOS simulator (`simctl log stream`) logs in one terminal UI, with the same keys and filters on both.
- App-only view: logs are followed by the app's UID on Android and executable on iOS, across restarts, with system noise hidden.
- Crash blocks for Java/Kotlin exceptions, native crashes, Swift fatal errors and uncaught exceptions.
- Targets: All, per OS and per device, with `Tab` and a target list; a notice for crashes on other devices.
- Android Studio style filter queries with completion and history; search; tag picker.
- Detail pane, copy, save, and an action palette (`:`).
- App picker with recent and running apps; switching apps mid-run.
- `--json` and `--plain` output, `--file` input, `--launch`.
- `--last 5m` prints recent logs and exits; `--for 30s` follows for a while and exits. Both are meant for scripts and coding agents.
- A guide for coding agents (`docs/agents.md`), examples in `mobicat --help`, and a Claude Code plugin with a mobicat skill.
- The last 5 minutes of logs on iOS too (`log show`), not only on Android.
- Config files (`~/.config/mobicat/config.toml`, `.mobicat.toml`) with profiles.

[Unreleased]: https://github.com/yamamoto7/mobicat/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/yamamoto7/mobicat/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/yamamoto7/mobicat/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/yamamoto7/mobicat/releases/tag/v0.1.0
