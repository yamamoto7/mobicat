# Architecture (draft)

> Draft. The command name is `mobicat` (decided 2026-11-08).
> Library versions and numbers were checked with the GitHub API on 2026-11-07.

## 1. Language: Rust

Premise: support macOS, Linux, and Windows. Pick the best option for the product, not the one the developer knows best.

| Aspect | Rust (Ratatui) | Go (Bubble Tea) | Others (Swift, Kotlin, TS, Python) |
|---|---|---|---|
| Rendering high-volume logs | ◎ Immediate mode that draws only the diff. You control how drawing happens | △–○ Calls View on every message. v2 has reports of performance regressions (#1481, #1857) | Swift ◎, others △ |
| Parsing and filtering speed | ◎ `regex` and `serde_json` are fast, and there is no GC | ○ RE2 is safe but slower | — |
| Buffers of 100k+ entries | ◎ Memory can be packed tightly | ○ GC overhead | — |
| Windows terminals | ◎ crossterm has strong support | ○ | — |
| Distribution | ○ Build per OS. cargo-dist produces GitHub Releases, brew, and shell / PowerShell installers | ◎ One machine can build for all OSes | Needs a runtime, or OS support is limited |
| Prior log tools | rogcat (logcat), hl ★3.3k, tailspin ★8.0k, yazi ★43k, gitui ★23k | lazylogcat, k9s, lazygit | toolong (Python) |
| iOS devices (Phase 2) | idevice ★570 (v0.1.68): syslog_relay, os_trace_relay, crashreport, RSD/XPC, **in-process tunnel** | go-ios ★2.3k: ostrace, syslog. iOS 17+ needs a sudo tunnel | pymobiledevice3 (Python) |
| Development speed | △ Slow builds, more code to write | ◎ | — |

**We use Rust.** The core job of this tool is to ingest, filter, and draw thousands of log lines per second without lag. Rust has the most headroom there. Many prior log tools are also written in Rust.

- cargo-dist and per-OS CI absorb the distribution cost.
- For iOS devices, idevice may have a tunnel that works without sudo. That could give users a better experience (unverified). It is younger than go-ios, so we check it before starting Phase 2. If it does not work, we call the go-ios CLI or pymobiledevice3 as an external command.

## 2. Technologies

| Role | Choice | Reason / alternatives |
|---|---|---|
| Toolchain | stable, edition 2024. Set an MSRV and check it in CI | |
| TUI | **ratatui 0.30** + **crossterm** | The standard combination, strong on Windows |
| List rendering | Custom widget (draws only visible rows, uses `StatefulWidget`) | Do not pass all rows to ratatui's `List` / `Paragraph` |
| Async | **tokio** (process, sync, time) | Starting and stopping external commands, `select!` over several streams. idevice also uses tokio, so we match it |
| CLI | **clap 4** (derive) + **clap_complete** | Shell completion. Completes app IDs from connected devices dynamically |
| JSON | **serde** + **serde_json** | Measure in M0. If too slow, consider `sonic-rs` / `simd-json` |
| Config | **toml** + serde, custom layer merging | user < repository < environment variables < flags. Warn on unknown keys |
| Time | **jiff** | Handles time zones (iOS `+0900`, logcat timestamps without a year) |
| Query | Custom lexer and recursive-descent parser → evaluation tree | The grammar is small, so no dependency |
| Regex | **regex** | Linear time, no slowdown on hostile input |
| Strings | Intern tag and subsystem. Message body is `Box<str>` | Keeps memory low even with a 100k-entry buffer |
| Clipboard | Write OSC52 directly → fall back to **arboard** | Works over SSH |
| Errors | **thiserror** (library crates), **anyhow** (binary) | |
| The tool's own logs | **tracing** to a file (`--debug-log`) | Keeps the TUI screen clean |
| Android | Call `adb` as an external command | Uses the existing ADB server and `ANDROID_SERIAL` as-is |
| iOS simulator | Call `xcrun simctl`. plist via `plutil -convert json` | macOS-only feature, so only this part uses `cfg(target_os = "macos")` |
| iOS devices (Phase 2) | **idevice** (MIT) | See the note above |

## 3. Data flow

```
 adb logcat ─┐                                       ┌─> TUI (every 30–60ms, draw if changed)
 simctl log ─┼─> source ─> parse ─> pipeline ─> store ─┤
 --launch ───┤   (tokio task)    - group into blocks └─> --json / --plain (stream as-is)
 file/stdin ─┘                   - crash detection, framework detection
                                 - app-only check (mark only, never drop)
```

- One source task per device. It handles external command exit, device disconnect, and reconnect. Child processes use `kill_on_drop` and are always cleaned up on Ctrl-C.
- Lines are read with a limit (1MB per line). Invalid UTF-8 is not dropped; it is read with replacement characters.
- The store holds a ring buffer of entries (default 100,000) and a list of indices that pass the filter. When the filter changes, the list is rebuilt on another thread and swapped in when done.
- The render loop runs on a dedicated thread separate from tokio. It handles crossterm input, periodic drawing, and checking the store's generation number.
- `--json` / `--plain` use the same pipeline without the TUI. Tests also use this path.

## 4. Directory layout (Cargo workspace)

```
crates/
  mobicat/            Binary. clap definitions, startup
  mobicat-core/       LogEntry, levels, pipeline, query, store (OS-independent; the main test target)
  mobicat-sources/    android, iossim, file, launch, device detection
  mobicat-tui/        ratatui screens, virtual list, detail pane, pickers, key bindings
testdata/             Recorded logcat text and log stream ndjson
```

- A separate `mobicat-core` makes builds faster and tests lighter. It also leaves room to ship it as a library for agents.
- Only `mobicat` is published to crates.io (the others are `publish = false`; revisit if needed).

## 5. Testing

- `mobicat-core`: table-driven tests that feed recorded `testdata/`, plus **insta** snapshots.
- Parser and query: **cargo-fuzz** (no crash on broken lines, long lines, or odd UTF-8) and proptest.
- TUI: compare screens drawn with ratatui's `TestBackend` against insta snapshots.
- `replay` source: replays a recording with its original timing. Also used for performance measurement and recording demo GIFs.
- Performance: **divan** or criterion, measuring ingest-to-draw at 2,000 lines/s and 10,000 lines/s.
- Runner: **cargo-nextest**.

## 6. CI and distribution

- GitHub Actions on three OSes.
  - ubuntu: fmt, clippy (warnings as errors), tests, **cargo-deny** (licenses and vulnerabilities).
  - macos: the same, plus integration tests for code that uses `xcrun` (skipped if no simulator).
  - windows: build and Android-side tests.
- Distribution: **cargo-dist (dist) v0.34**
  - Targets: `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-musl` (static), `x86_64-pc-windows-msvc`.
  - Outputs: GitHub Releases, shell / PowerShell installers, a formula for the Homebrew tap (`yamamoto7/homebrew-tap`).
  - Also support `cargo install mobicat` and `cargo binstall mobicat`.
  - Later: homebrew-core, winget, Scoop, AUR, Nix.
  - Homebrew 6 requires users to run `brew trust` first, so the README says so.
- Supported OSes: Android side on macOS, Linux, and Windows. iOS side on macOS only (needs `xcrun`).

## 7. Open questions

Decided: the command and crate name is `mobicat`; the MSRV is Rust 1.88 (checked in CI).

- Whether to switch to a faster JSON parser, based on M0 results
- Phase 2: whether idevice's tunnel can connect to iOS 17+ devices without sudo
