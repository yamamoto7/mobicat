# Changelog

All notable changes are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

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
- The last 5 minutes of logs on iOS too (`log show`), not only on Android.
- Config files (`~/.config/mobicat/config.toml`, `.mobicat.toml`) with profiles.
