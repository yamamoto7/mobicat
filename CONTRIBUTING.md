# Contributing to mobicat

Thanks for helping. Bug reports, ideas and pull requests are all welcome.

## Reporting a bug

Open an [issue](https://github.com/yamamoto7/mobicat/issues/new/choose) and include:

- `mobicat --version`, your OS and terminal
- The platform and device (for example "Pixel 8 emulator, API 35" or "iPhone 17 Pro simulator, iOS 26.5")
- What you ran, what you expected, and what happened

Log lines that were parsed or classified wrongly help the most. Save them with
`Ctrl-s` in the TUI, or with `adb logcat -v threadtime` /
`xcrun simctl spawn booted log stream --style ndjson`, and remove anything private.

## Building

You need Rust 1.88 or later.

```sh
cargo build
cargo test --workspace
cargo run -- --file testdata/android/logcat-threadtime.txt   # try the TUI without a device
```

Before you open a pull request, run what CI runs:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Layout

| Path | What |
|---|---|
| `crates/mobicat-core` | Entries, the store, the query language, crash grouping, output formats |
| `crates/mobicat-sources` | adb, iOS simulator and file sources |
| `crates/mobicat-tui` | The terminal UI |
| `crates/mobicat` | The command: flags, config files, wiring |
| `testdata/` | Recorded and hand-written logs used by tests |
| `docs/` | Requirements, architecture and technical notes |
| `demo/` | The demo app and the scripts that record the README GIF |

## Tests

- Parsers and classification are tested against the files in `testdata/`. When you add a fixture, sanitize it (app ID `com.example.app`, user name `dev`) and describe it in `testdata/README.md`.
- The TUI uses [insta](https://insta.rs) snapshots. After an intended UI change, review and accept them with `cargo insta review` (install with `cargo install cargo-insta`).

## Pull requests

- Keep each pull request to one change, and add tests for behavior changes.
- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `docs:` …), in English.
- For a larger change (a new source, a new screen), open an issue first so we can agree on the approach.

By contributing, you agree that your contributions are licensed under the [Apache License 2.0](LICENSE).
