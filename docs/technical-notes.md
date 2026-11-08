# Technical notes

Items marked "(unverified)" are based on memory or research notes and have not been run locally yet. Check them one by one before implementation.

## Verification results (2026-11-07)

Environment: macOS 26.6.2, Xcode 26.5, iOS 26.5 simulator (iPhone 17 Pro), Android emulator API 35.

### Android
- `adb logcat --uid=<UID>`: UID filtering works on the device side. The UID does not change when the app restarts, so **we can follow across restarts without tracking PIDs**. Subprocesses (`:remote`, etc.) share the UID, so they are included. WebView sandbox processes use an isolated UID (`u0i*`), so they are not.
- Getting the UID: `adb shell cmd package list packages -U <package>` → `package:com.example uid:10210`.
- `-v threadtime,uid` format: `11-07 22:22:37.468 10210  3727  3727 I Tag: message`.
- `Start proc <pid>:<package>/u0a210 for ...` lines still come from ActivityManager on API 35 (the pidcat approach also works). `Force stopping <package>` shows termination.
- Whether some old API levels lack `--uid` is unverified. If it is missing, switch to watching `Start proc` and tracking the PID with `pidof`.
- The app's process also emits system logs from ART, nativeloader, and so on (they must be told apart by tag).

### iOS simulator
- `log stream --style ndjson --level debug` works. One JSON object per line. Fields: `timestamp` (`2026-11-07 22:16:43.066999+0900`), `messageType` (Debug/Info/Default/Error/Fault), `eventType` (`logEvent` / `activityCreateEvent`; the latter has no messageType, so drop it), `subsystem`, `category`, `processID`, `processImagePath`, `senderImagePath`, `eventMessage`, and more.
- stderr shows `getpwuid_r did not find a match for uid 501`, which is harmless.
- Confirmed that `--predicate 'process == "<CFBundleExecutable>"'` **keeps following across restarts** (PID 80225 → 80248).
- bundle ID → executable name: convert to JSON with `xcrun simctl listapps <udid> | plutil -convert json -o - -` and read `CFBundleExecutable`. **Different apps can share an executable name** (for example, `com.example.flow` and `com.example2.flow` are both `Flow`). Tell them apart by checking whether `processImagePath` is under the app's bundle path (`simctl get_app_container <udid> <bundle> app`).
- **Very noisy**: one app's process produced 15,337 lines in 8 seconds. Only 28 of them (0.2%) came from the app's own binary (`senderImagePath` inside the .app). The top sources were com.apple.network (5,225), com.apple.defaults (5,090), and com.apple.CFBundle. "Only the app's own logs" must be the default.
- Swift's `print()` (stdout) does not appear in `log stream`. It can only be captured when the tool launches the app with `xcrun simctl launch --console-pty <udid> <bundle>`.

## Android

- Capture: `adb logcat`. Add `adb -s <serial>` to target a device.
  - Output format: `-v threadtime` (default). There are modifiers such as `-v epoch` and `-v uid`. Choose an easy-to-parse format (unverified).
  - Filter by PID: `adb logcat --pid=<pid>`. Get the PID with `adb shell pidof -s <package>` (unverified: behavior on old API levels).
  - Crashes: the `-b crash` buffer, or `FATAL EXCEPTION` with the `AndroidRuntime` tag.
- **Following restarts**: pidcat watches ActivityManager's `Start proc <pid>:<package>` lines and updates a PID map (unverified: wording on recent Android). Another option is to call `pidof` periodically and watch for changes.
- Device changes: `adb track-devices`.
- References: pidcat source (Python, Apache-2.0), rogcat (Rust, MIT), lazylogcat (MIT).

## iOS simulator

- List booted simulators: `xcrun simctl list devices booted -j`.
- Capture: `xcrun simctl spawn <udid|booted> log stream --style ndjson --level debug --predicate '<predicate>'`
  - `--style ndjson` emits one JSON object per line (unverified: which macOS versions support it, and how it differs from `--style json`).
  - Filter examples: `process == "MyApp"`, `subsystem == "com.example.myapp"`, `category == "network"`.
  - Filtering by `process == ...` is expected to keep following the app across restarts (unverified).
  - Available fields: timestamp, messageType (Default, Info, Debug, Error, Fault), subsystem, category, processImagePath, eventMessage, and more (unverified).
- Look up the process name from a bundle ID: `xcrun simctl listapps <udid>` (outputs plist, unverified). Or `xcrun simctl get_app_container`.

## iOS devices (Phase 2, capture method needs research)

- Candidates (all unverified):
  - `idevicesyslog` (libimobiledevice): via syslog relay. Structured OSLog data may be lost.
  - `pymobiledevice3 syslog live` and os_trace tools: said to stream structured logs. Adds a Python dependency.
  - `xcrun devicectl device process launch --console` (Xcode 15+): only stdout and stderr of a process you launch yourself.
  - `log collect --device`: collects an archive; it does not stream in real time.
- Reports say the connection method changed in iOS 17+ (RemoteXPC and tunnels). Device support could take a lot of work, so it was left out of the MVP.

## Design outline (draft)

```
sources/        # capture adapters: android(adb) / iossim(simctl) / iosdevice(later)
  → convert to a common LogEntry
     {platform, device, app, pid, time, level, tag/subsystem, category, message, raw}
pipeline/       # filtering, crash detection, PID tracking
ui/             # Ratatui: list, targets, detail, filter editing
output/         # --json (NDJSON) / --plain (for pipes)
config/         # per-project config file (name TBD)
```

- Align levels across the two OSes: map Android's V/D/I/W/E/F and iOS's Debug/Info/Default/Error/Fault to a common 5-level scale.
- Support streaming with `--json` / `--plain` without the TUI from the start. This makes testing easier and lets agents call the tool.
- Testing: do not depend on devices. Build an adapter that feeds recorded logcat output and ndjson, and use those recordings as fixtures.

## Development environment

- Requires Android SDK platform-tools (adb) and the Xcode command line tools.
- CI: Android-side parsing tests on a Linux runner, iOS-side parsing tests on a macOS runner. No devices; tests use fixtures.

## M0: rendering performance prototype (2026-11-07)

We streamed synthetic logs with `mobicat dev synth` (a hidden command) and measured for 10 seconds each in a 160×48 pseudo-terminal. With `--exercise`, it scrolls, pauses, and toggles the level filter automatically every 500ms. Environment: Apple Silicon Mac, release build.

| Condition | One draw (p50 / p99 / max) | Filter rebuild (max) | CPU (10 s) | Peak memory |
|---|---|---|---|---|
| 2,000 lines/s, 100k buffer | 0.83 / 1.78 / 2.5 ms | 0.31 ms (20k entries) | 0.29 s | 7.8 MB |
| 10,000 lines/s, 100k buffer full | 0.79 / 2.63 / 4.8 ms | 0.96 ms | 0.36 s | 23 MB |
| 50,000 lines/s, 100k buffer full | 1.05 / 2.65 / 3.1 ms | 1.15 ms | 0.59 s | 23 MB |
| 10,000 lines/s, 1M buffer full | 1.04 / 3.20 / 10.5 ms | 16.3 ms | 0.62 s | 191 MB |
| 10,000 lines/s, 60fps (16ms interval) | 0.67 / 2.54 / 10.2 ms | 1.20 ms | 0.54 s | 23 MB |

- The goal (key press to draw within 50ms at 2,000 lines/s) is met by a wide margin. Draw interval (33ms) + draw (under 3ms) gives about 36ms in the worst case.
- About 160–180 bytes per entry. The default 100k entries fit in about 20MB.
- At 1M entries, a filter rebuild takes up to 16ms. If queries get heavier in M3, move rebuilds to another thread (as planned in architecture.md §3).
- Draw time covers Rust computing the diff and writing it to the terminal. It does not include the terminal app's own rendering cost.

## M1: capture with real data (2026-11-07)

We ran `mobicat <app> --json` against an Android emulator (API 35) and an iOS 26.5 simulator at the same time, and restarted the app twice on each OS. Both kept following across restarts and printed the `process started (pid N)` separator line. No child processes are left behind after `q` in the TUI or Ctrl-C in `--json`.

Findings during implementation:

- **logcat's `-t` / `-T <count>` counts all lines before the `--uid` filter.** Combining `--uid` with `-t 5` gives 0 lines. Specify past logs by time, not count (pass 5 minutes before the device's `date +%s` as `-T <epoch>`; `--clear` starts from now).
- **`cmd package list packages -U <pkg>` returns substring matches** (`com.example.app` also returns `com.example.app.stg`). Pick the exact match. When the app is not installed, the output is empty and the exit code is 0.
- **`log stream` ndjson escapes `/` in paths as `\/`.** Borrowing as `&str` in serde fails on values with escapes, so we take `Cow<str>` (regression test included).
- **Sending SIGKILL to `xcrun simctl spawn` leaves `log stream` running inside the simulator** (it does not exit until the next write). SIGTERM / SIGINT propagate inside. Child processes are stopped with SIGTERM, then SIGKILL after 1 second.
- **Fix to the iOS app-only check**: Xcode Previews stubs (subsystem `com.apple.Previews.StubExecutor`) log from a binary inside the .app. The two rules "inside the .app" and "UUID matches" are not applied to lines whose subsystem starts with `com.apple.`.
- The recorded app did not emit its own os_log (0 app lines). The app's own logs, React Native, Kermit, and Swift Fatal error are verified with hand-made fixtures in the same format as the recordings (`testdata/ios/sim-handmade.ndjson`).
- With `--file`, we cannot tell which app a line belongs to, so all lines are treated as app-only.

Not done yet:

- Reading past logs on iOS (expected to work with `log show --last`; unverified)
- PID tracking on old Android versions without `--uid`
- `--launch` (M4), crash blocks and framework detection (M3)

## M2: TUI basics (2026-11-07)

- **Cursor**: no cursor during follow. The first `k` selects the newest line and pauses, then moves up from there. Moving below the newest line returns to follow. Pausing keeps the screen position (the top line is tracked even during follow).
- **Variable-height list**: expanded blocks and wrapping make one entry span several rows. During follow, rows stack upward from the newest line; while paused, they stack downward from the remembered top. The cursor row is always on screen, and when there are rows above, no blank space is left at the bottom.
- **Multi-line blocks**: logcat splits one message containing newlines into several lines with the same header (pid, tid, level, tag, time). Confirmed by emitting a 3-line message with `adb shell log`. We merge them back into one entry (`Grouper` in `mobicat-core`, max 2000 lines). If no continuation line arrives within 30ms, the entry is finalized. On iOS, one record already contains the newlines, so it is used as-is.
- **Cancel-safe line reading**: for the 30ms wait above, `LineReader` no longer discards a partially read line (regression test included).
- Tabs in stack traces are drawn as four spaces, because tab width varies by terminal.
- Detail pane: bottom 45% if the body is 24 rows or more, full screen if smaller. Esc closes it (Esc with nothing open quits).
- The mouse is not captured, so terminal text selection keeps working. In many terminals the wheel arrives as up/down keys (alternate scroll).
- Screen tests: `TestBackend` and six insta snapshots (follow, all lines and paused, expanded block, split detail, full-screen detail, help). Times are drawn in UTC.
- Performance (same conditions as M0): draw p99 is 2.9ms at 50,000 lines/s. Same level as M0 even after rebuilding the list.

## M3, M4: query, crashes, config, --launch (2026-11-08)

Confirmed (emulator API 35, iOS 26.5 simulator):

- **Android noise**: for one app over the last 5 minutes, 22 lines in total, 5 lines in app-only. Hidden were EGL_emulation, TrafficStats, and ART lines. Ad SDK lines remained.
- **Crashes**: we emitted a 4-line `FATAL EXCEPTION` with `adb shell log` and read it with `--file`. It became one block marked `✖ CRASH` and could be selected with `is:crash`.
- **`--launch`**:
  - On Android, the app relaunched and `process started` appeared.
  - On iOS, stdout / stderr lines that do not appear in `log stream` (objc warnings, etc.) came in as `source=stdout`.
  - The `<bundle>: <pid>` line that `simctl launch --console-pty` prints itself is buffered through a pipe and arrives late (sometimes not until exit). We do not rely on this line and drop it when it arrives.
- **`simctl launch` left running**: once, `simctl launch` was still running after mobicat exited. SIGTERM / SIGINT / SIGHUP each stopped it correctly on their own. After changing how the pid line above is handled, we tried twice on each OS and nothing was left. The cause is unclear (keep watching).
- **Config**: found `.mobicat.toml` from a subdirectory, resolved `extends` to `(parent) (child)`, and warned on unknown keys.
- **App picker**: showed apps installed on both OSes with `A i` markers, narrowed by typing, and opened with Enter.

Performance (same conditions as M0):
- At 50,000 lines/s, draw p99 is 4.2ms. With three lists kept (one per OS), it rose slightly from M2's 2.9ms, but stays far from the goal (50ms).
- With 100k entries, a rebuild for a query with a regex takes 3–4ms. Applying it on every keystroke causes no lag.
