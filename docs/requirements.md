# Requirements (MVP)

> Draft. The command name is `mobicat` (decided 2026-11-08).
> "(unverified)" marks items to check locally before implementation.

## 1. Goal and scope

View Android and iOS simulator app logs in one terminal UI. When you name an app, mobicat keeps following it across restarts, shows only the app's own logs by default, and makes crashes stand out.

| Target | MVP |
|---|---|
| Android emulators and devices (adb) | In scope |
| iOS simulators (simctl) | In scope |
| iOS devices | Out of scope (Phase 2) |
| Reading from file or stdin (logcat text, `log stream` ndjson, this tool's NDJSON) | In scope |
| GUI, web UI, log persistence, network logs, AI summaries | Out of scope |

## 2. Command line

```
mobicat [APP_ID ...] [flags]
```

- `APP_ID`: package name / bundle ID. If both OSes use the same ID, one is enough. If they differ, use `--android <pkg> --ios <bundle>`.
- If omitted: use `app` from the config file. If that is also missing, show a picker of apps on connected devices.
- Devices: auto-detect connected Android devices (`adb devices`) and booted simulators (`simctl list devices booted`). If there are several, show a picker. Specify with `--device <serial|udid>` (repeatable; `-s`/`--serial` also work, as in pidcat).

| Flag | Description |
|---|---|
| `--json` | Do not open the TUI; stream NDJSON (§10) |
| `--plain` | Do not open the TUI; stream a colored one-line format (no color when piped) |
| `-q, --query <QUERY>` | Initial filter (grammar in §7) |
| `-p, --profile <NAME>` | Use a saved filter from the config file |
| `-l, --level <LEVEL>` | Minimum level (`v` `d` `i` `w` `e` `f` or the full names). Alias `--min-level`, as in pidcat |
| `-t, --tag <TAG>` / `-i, --ignore-tag <TAG>` | Show only / hide these tags, repeatable. Same as `-q tag:X` / `-q -tag:X`, for pidcat users |
| `--all` | Turn off filtering to the app's own logs (app-only) |
| `--launch` | The tool relaunches the app and also captures stdout/stderr (iOS. §4.2) |
| `--clear` | Do not read past logs at start (default: the last 5 minutes. logcat's count option is applied before `--uid`, so we specify a time instead. iOS uses `log show`, which keeps Info and above, not Debug) |
| `--last <DURATION>` | Print the logs of the last DURATION (`30s`, `5m`, `1h`) in time order and exit (Android `logcat -d`, iOS `log show`). With `--for`, keep following after that. For scripts and agents |
| `--for <DURATION>` | Follow for DURATION, then exit. Starts with new lines only unless `--last` is given. With `--launch`, captures a startup |
| `--file <PATH>` / `-` | Read from a file or stdin. Format is auto-detected |
| `--color auto\|always\|never` | Also respects `NO_COLOR` |

## 3. Common log format (LogEntry)

```
platform   android | ios
device     serial / UDID and display name
app        package name / bundle ID
pid, tid
time       device local time (nanosecond precision)
level      V D I W E F (table below)
tag        Android: tag / iOS: category, else the last part of subsystem
subsystem, category   iOS only
library    iOS: basename of senderImagePath
source     log | stdout | stderr | system (separator lines, etc.)
framework  native | react-native | flutter | kmp | capacitor
crash      ID if this is a crash block
message    body (may be multi-line)
raw        original line / original JSON
```

Level mapping (iOS has no Warning):

| Common | Android | iOS |
|---|---|---|
| V | V | — |
| D | D | Debug |
| I | I | Info, Default |
| W | W | — (RN's warn becomes Info, so it cannot be told apart) |
| E | E | Error |
| F | F / A | Fault |

## 4. Log capture

### 4.1 Android

- Get the UID with `adb shell cmd package list packages -U <pkg>` and stream with `adb logcat --uid=<UID> -v threadtime,uid -b main,system,crash`. The UID does not change across restarts, so we can keep following without tracking PIDs (confirmed on API 35).
- Where `--uid` is not available, switch to watching `Start proc <pid>:<pkg>/` lines and tracking the PID with `pidof` (which API levels lack it: unverified).
- When the PID changes under the same UID, insert a `── process started (pid 3990) ──` separator line.
- Watch device plug/unplug with `adb track-devices`. On disconnect, wait for reconnect.

### 4.2 iOS simulator

- Get `CFBundleExecutable` from `simctl listapps`, and the .app path from `simctl get_app_container <udid> <bundle> app`.
- Stream with `simctl spawn <udid> log stream --style ndjson --level debug --predicate 'process == "<exe>"'`. This keeps following across restarts (confirmed). Drop `activityCreateEvent`.
- To exclude other apps with the same executable name, check that `processImagePath` is under the .app.
- **App-only check** (on by default. Decided inside mobicat; all lines stay in the buffer, so toggling takes effect immediately). Keep lines that match any of these:
  1. `senderImagePath` is `libswiftCore.dylib` (`fatalError`, etc.)
  2. `subsystem == "com.facebook.react.log"` (React Native)
  3. `subsystem` starts with the bundle ID
  4. `senderImagePath` is inside the .app (the app binary and bundled frameworks). Exclude lines whose subsystem starts with `com.apple.` (Xcode Previews stubs log from inside the .app)
  5. `processImageUUID == senderImageUUID` (same exclusion as 4)
  - Reference: the simulator predicate in Flutter's official tooling.
- `--launch`: launch with `simctl launch --console-pty --terminate-running-process <udid> <bundle>` and merge its output into the same timeline as `source=stdout`. Swift's `print()` and Capacitor's `console.log` can only be captured this way.
- Without `--launch`, if the app has started but no app-only logs arrive for a while, the status line suggests "use `--launch` if you use print()".

### 4.3 Android app-only

The app's process also emits logs from ART, nativeloader, and so on. We keep a preset list of system tags and hide them in app-only mode. The list can be extended or reduced in config.

## 5. Framework detection

| Framework | Android | iOS |
|---|---|---|
| React Native | tag `ReactNativeJS` | subsystem `com.facebook.react.log` |
| Flutter | tag `flutter` | message starts with `flutter: ` |
| KMP (Kermit) | message shape | subsystem and category are empty, message is `<emoji> (Tag) msg` |
| Capacitor | tag `Capacitor/Console` | stdout `⚡️  [log] - ` |

Detected lines get a small marker. `flutter: ` and Kermit's emoji are stripped from the message and moved into tag and level. Flutter's `dart:developer log()` does not appear in either OS's log, so it is out of scope; the README says so.

## 6. Screen

```
┌ mobicat  com.example.app ─ A: Pixel 9 (emulator-5554)  i: iPhone 17 Pro ─────────────┐
│22:17:05.123 A  I        MainActivity onCreate                                    │
│22:17:05.130 i  I             network GET /api/items 200                          │
│22:17:05.131 i  D                     decoded 20 items                            │
│22:17:06.002 A  E              OkHttp timeout after 10s                 +12 lines │
│22:17:06.410 i ▌F ✖ CRASH   libswiftCore Fatal error: Unexpectedly found nil ...  │
│            ── process started (pid 3990) ──                                      │
├──────────────────────────────────────────────────────────────────────────────────┤
│ FOLLOW  level≥D  q: tag:network -tag:Chatty  app-only  ✖ 1 crash   A 1,204  i 388 │
│ / filter  ]c crash  Enter detail  Space pause  ? help                             │
└──────────────────────────────────────────────────────────────────────────────────┘
```

- **The default is a single stream**, with an OS marker (`A` / `i`) on each line. Lines are in arrival order.
- **Line layout**: `time  OS  level  tag (right-aligned, omitted when repeated)  message`. Level is shown by both letter and color.
- **Toggleable columns**: PID:TID, subsystem, category, library, framework marker (via keys and config).
- **Wrapping**: aligned to the message column. `Ctrl-w` toggles wrap and truncate.
- **Multi-line blocks** (stack traces, etc.): merged into one entry at ingest, collapsed as `+N lines` in the list. Crash blocks start expanded.
- **Detail pane** (`Enter`): bottom half when the terminal is wide enough, full screen when narrow. Shows all fields and raw data. From there you can filter by / exclude this tag, copy, and see surrounding context.
- **One view.** You switch what it shows (the **target**): "All" (all devices merged by time), an OS, or a single device. No top/bottom split (dropped on 2026-11-08).
- Header: `mobicat · app ID`. Below it, a **tab row**: `All 7  ● Pixel 9 4  ● iPhone 17 Pro 3 ✖1`. The current target uses the accent color and an underline. New crashes on targets you are not viewing show as a red `✖N`. Disconnected devices show `○`, dimmed.
- Status line: `● FOLLOW · ✖ 2 · level ≥ V · app-only · q: …`, with the visible count on the right. Bottom line: hints for keys available in the current state (changes per state).

## 7. Filter

The filter is **one query string**. Pickers and detail-pane buttons only rewrite this string. The grammar follows Android Studio Logcat, with iOS fields added.

```
level:warn            this level and above
tag:OkHttp            substring match.  tag~:^Ok  for regex.  -tag:Chatty  to exclude
message:timeout       (bare words also match message)
subsystem: category: library: pid: package:
os:android | os:ios
is:crash  is:stacktrace  is:stdout
fw:react-native | fw:flutter | fw:kmp | fw:native
age:5m
```

- The same field repeated means OR; different fields mean AND. `|`, `&`, and parentheses are also supported.
- `/` starts editing. Results update with a short delay as you type. `Enter` confirms, `Esc` cancels.
- `x`: toggle between hiding non-matching lines and only highlighting matching lines.
- Syntax errors are shown below the input, with a position.

## 8. Keys

| Key | Action |
|---|---|
| `j` `k` / `↑` `↓` | Move the cursor by one entry (`k` during follow selects the newest line and pauses. Moving below the newest line returns to follow) |
| `PgDn` `PgUp` | Move half a screen (`Ctrl-d` was removed because it is used for quit) |
| `g` / `G` | Top / bottom (`G` resumes follow) |
| `Space` | Pause / resume follow |
| `/` | Edit filter |
| `f` / `n` `N` | Search (view unchanged), next / previous |
| `x` | Toggle hide / highlight only |
| `-` `+` | Lower / raise the minimum level |
| `t` | Tag picker (iOS: subsystem/category) |
| `a` | Toggle app-only |
| `Tab` / `Shift-Tab` | Next / previous target (All → each device) |
| `d` | Target list (tree: OS ▸ device). Selecting an OS row shows all devices of that OS together. "Change app…" opens the app picker |
| `1`–`9` | Apply a saved filter |
| `]c` `[c` / `]e` `[e` (`e` `E`) / `]w` `[w` (`w` `W`) | Next / previous crash, error, warning. Pressing `]` shows the possible next keys |
| `Enter` / `o` | Detail pane / expand or collapse a block |
| `Ctrl-w` / `T` | Wrap / time format |
| `y` / `V`→`y` / `Y` | Copy line / copy range / copy all visible lines |
| `Ctrl-s` | Save visible lines to a file |
| `c` | Clear the logs on screen (and the buffer). Confirm with `y` |
| `:` | Command list. Search by name and run. Each row also shows its key |
| `?` | Key list (grouped) |
| `Esc` / `q` | Back: closes one layer at a time, in this order: range selection → detail pane → pause. `Esc` never quits. When there is nothing to go back from, `q` acts like the first `Ctrl-C` |
| `Ctrl-C` `Ctrl-C` / `Ctrl-D` `Ctrl-D` | Quit. The first press shows "Press Ctrl-C again to exit"; a second press within 2 seconds quits |

- `Ctrl-j` and `Ctrl-k` are not used (some terminals cannot tell them apart).
- The mouse is not captured (so terminal text selection keeps working). In many terminals the wheel arrives as up/down keys.

## 9. Crashes

| OS | What is detected |
|---|---|
| Android | `AndroidRuntime` `FATAL EXCEPTION` and the stack that follows, the crash buffer, native crashes (blocks starting with `*** *** ***` from `DEBUG`) |
| iOS | Fault level, `libswiftCore` `Fatal error`, `Terminating app due to uncaught exception` (where it appears: unverified) |

- Merge into one block and make it stand out with a red band and a marker at the start of the line (recognizable by shape even without color).
- Show the count in the status line. `]c` jumps to the next crash. `y` copies the whole block.
- ANRs are emitted by system_server (a different UID), so they are not in the `--uid` stream. Whether to catch them with a second stream (`ActivityManager` only) is decided later.

## 10. Output (`--json` / `--plain`)

- `--json`: one object per line. Keys match §3. `schema` holds a version number, bumped on breaking changes.
- `--plain`: same as the TUI's one-line layout. `--query`, `--profile`, and `--all` work in both.
- `--last` and `--for` print without the TUI and exit, so a coding agent can run mobicat like any command. They print `--plain` lines unless `--json` is given.
- Copy and save pass the message as-is, without padding or wrapping. The clipboard uses OSC52, falling back to `pbcopy` / `wl-copy` / `xclip`.

## 11. Config file

- Two layers: user config `~/.config/mobicat/config.toml` (same location on macOS) and `.mobicat.toml` at the repository root (shared by the team).
- Precedence: user < repository < environment variables < command-line flags.
- Format: TOML (query strings contain many `:` and `-`, and YAML would need quotes in many cases).

```toml
app = "com.example.app"           # when both OSes use the same ID
[apps]
android = "com.example.app.dev"
ios     = "com.example.app"

[defaults]
level = "debug"
query = "-tag:Chatty"

[profiles.network]
query = "tag:OkHttp | category:network"

[profiles.network-errors]
extends = "network"
query   = "level:error"

[noise.android]
tags = ["nativeloader", "ApplicationLoaders"]   # add tags to hide in app-only mode
```

## 12. Performance

- Goal: no input lag at a combined 2,000 lines/s (key press to draw within about 50ms).
- Ingest never stops. Ring buffer of entries (default 100,000, configurable).
- Draw every 30–60ms, only when something changed. Do not draw per line.
- A custom list that draws only visible rows (do not pass all rows to ratatui's List). Architecture is in `docs/architecture.md`.
- Changing the filter recounts the whole buffer in the background.

## 13. Colors

- Default is ANSI 16 colors only. Do not paint the full-screen background (readable on both light and dark terminals).
- Do not use blue, bright yellow, or bright black for message text (unreadable on some terminals).
- Tag colors come from a hash of the name, excluding red and yellow (confusable with errors).
- Respect `NO_COLOR` and `--color never`.

## 14. Build order

| Stage | Content | Done when |
|---|---|---|
| M0 | Performance prototype: a Ratatui list fed synthetic logs at 2,000 lines/s | Target speed is reached. If not, rethink the rendering approach |
| M1 | Capture (Android, iOS simulator, file), LogEntry, `--plain` / `--json` | `--json` keeps following apps on both OSes across restarts. Tests pass on fixtures |
| M2 | TUI basics: list, follow, header and status, detail pane, multi-line blocks | Usable daily on real apps |
| M3 | Filter grammar, app-only, crashes, framework detection | All keys work |
| M4 | Config file, `--launch`, pickers, copy and save (a top/bottom split was later dropped) | MVP complete |

## 15. Unverified (check before implementation)

- The oldest API level that supports `logcat --uid`
- How far back to read past logs at start (handling of `-T`)
- Whether iOS `Terminating app due to uncaught exception` goes to os_log or stderr
- Which messageType and sender Flutter lines have in ndjson
- Whether RN's sender changes between static pods and `use_frameworks!`
- Whether a multi-line message is one event in `log stream`
- How Kermit's `<private>` appears on the simulator

## 16. Decisions made during implementation and changes from the spec (M3, M4, 2026-11-08)

Details decided while implementing, and changes from the sections above. This section takes precedence over the sections above.

- **Devices**: instead of picking one in a picker when there are several, use all connected devices and simulators (narrow with `--device`). The tool shows both OSes at once, so we skip the selection step.
- **App picker**: when there is no app ID and no `app` in the config file, show apps installed on connected devices (Android: `pm list packages -3`, iOS: user apps) in one list. Apps installed on both OSes come first. With `--json` / `--plain` (not a terminal), do not show the picker; exit with an error.
- **`--launch`**: also works on Android (force-stops the app and opens the launcher Activity). On iOS, relaunch with `simctl launch --console-pty --terminate-running-process` and merge stdout as `source=stdout` (tag `stdout`). With `--launch`, past logs are not read.
- **Query**:
  - Bare words are always AND (`http failed` matches lines containing both). Repeated `message:` is OR.
  - Values without uppercase letters are case-insensitive (smartcase). Regex (`~:`) is used as-is.
  - mobicat's own lines (restart, disconnect, warnings) are never hidden by any filter in the TUI. With `--json` or `--plain` and a query, they must match the query too (`-q is:system` selects them), so scripts get exactly what they ask for.
  - `is:` accepts `crash`, `stacktrace`, `stdout`, `system`.
  - A config profile and the command-line query are each wrapped in parentheses and ANDed (so `|` does not leak between them).
- **Android app-only**: hide tags the OS emits in the app's process. The default list is `DEFAULT_ANDROID_NOISE` in `crates/mobicat-core/src/annotate.rs` (chosen from real API 35 data). Also hide lines where ART uses the process name (last 15 characters) as the tag. SDKs bundled with the app (Firebase, ads, etc.) are not hidden (the same idea as keeping frameworks inside the .app on iOS). Crashes are always kept.
- **Crashes**:
  - An iOS Fault alone is not a crash (Fault is also used for ordinary errors). A Fault containing `Fatal error:`, `Terminating app due to uncaught exception`, and `Fatal error:` on stdout are crashes.
  - Crash blocks start expanded and can be collapsed with `o`.
  - For Android native crashes (tombstones), the `DEBUG` `*** *** ***` line and the `libc` `Fatal signal` line get a marker. Each tombstone line has a different timestamp, so they are not merged into one block.
- **KMP detection**: on iOS, detect Kermit's `<emoji> (Tag) msg` shape and extract tag and level. On Android, Kermit uses its tag directly as the logcat tag, so it cannot be told apart.
- **Screen**:
  - A list is always kept per target (All, OS, device), so switching needs no rebuild wait. The store holds the device list and registers each device when first seen (`Store::add_device` can give it a name).
  - Each target remembers its pause position and selected line. The filter is shared across all targets.
  - Crashes on targets you are not viewing add `✖N` to the tab and show a notice ("✖ new crash on iPhone 17 Pro") once. Viewing All counts as having seen everything.
  - Device connect and disconnect are checked every 3 seconds with `adb devices` and `simctl list`. If a newly connected device has the app installed, capture starts right away. Emulators are shown by AVD name.
  - The selected line is shown with a bar at the left edge (`▌`) and a faint background. Only the terminal's 16 colors are used; the accent is cyan. Borders have rounded corners.
  - The app picker is split into "Recent", "Running now", and "Installed". The previous app is at the top, labeled "last". Typing switches to a flat list of matches only.
  - Input fields (`/`, `f`) have a border and show the match count at the top right. Completion candidates (key names, levels, tags actually seen in the stream, etc.) appear below; `Tab` inserts them in turn. `↑` `↓` browse history. History and recent apps are stored in `~/.local/state/mobicat/` (Windows: `%LOCALAPPDATA%\mobicat\`).
  - Filter by / exclude the selected line's tag with `=` / `!`.
- **Copy**: locally, use `pbcopy` / `wl-copy` / `xclip` / `xsel` / `clip.exe`; over SSH, use OSC 52 (up to about 100KB). For anything larger, save with `Ctrl-s`. The file is `mobicat-YYYYMMDD-HHMMSS.log` in the current directory.
- **Config file**: profiles are assigned to `1`–`9` in the order they appear in the file. Unknown keys are warned about and ignored; values of the wrong type are errors. On Windows the user config is `%APPDATA%\mobicat\config.toml`. `mobicat config` shows which files were read and the result.
- **First run (v0.1.2, 2026-11-10)**: when the app is not installed, the error lists the closest installed apps (IDs that contain what was typed, then misspellings within an edit distance of about a fifth of the length) and points at the picker. When nothing can be used, it says what to do: install platform-tools or set `ADB`; start an emulator or connect a device; accept USB debugging on an `unauthorized` device; reconnect an `offline` one; boot a simulator; select Xcode when only the Command Line Tools are selected (`unable to find utility "simctl"`).
