# mobicat

Android and iOS app logs in one terminal UI.

mobicat streams logs from Android devices and emulators (`adb logcat`) and iOS simulators (`simctl log stream`), merges them in time order, and shows only your app's own logs. It keeps following the app across restarts and makes crashes hard to miss.

[日本語](README.ja.md)

![mobicat showing logs from an Android emulator and an iOS simulator side by side, with a crash highlighted](docs/assets/demo.gif)

## Features

- **Both platforms in one view.** Logs from every connected Android device and booted simulator, in one timeline. Switch between All, one OS, or one device with `Tab`.
- **Your app only.** Android logs are taken by the app's UID, iOS logs by its executable. System noise is hidden by default (`a` shows everything). Restarts don't lose the app; a `── process started (pid N) ──` line marks them.
- **Crashes stand out.** Java/Kotlin exceptions, native crashes, Swift fatal errors and uncaught exceptions are grouped into one red `✖ CRASH` block with the stack trace. A crash on a device you aren't looking at shows up on its tab.
- **Android Studio filter syntax.** `tag:OkHttp level:warn -tag:Chatty`, plus `subsystem:`, `os:`, `fw:rn`, `age:5m` and more. Results update as you type, with completion and history.
- **Cross-platform frameworks.** React Native, Flutter, Kotlin Multiplatform and Capacitor logs are recognized (`fw:` in queries).
- **Scriptable.** `--json` prints NDJSON and `--plain` prints one line per entry, for `jq`, CI and AI agents.

## Requirements

- Android: `adb` (Android SDK platform-tools) on your `PATH`.
- iOS: macOS with Xcode (`xcrun simctl`). Simulators only for now; physical iOS devices are not supported yet.

## Install

Prebuilt binaries are not published yet. Build from source with Rust 1.88 or later:

```sh
cargo install --locked --git https://github.com/yamamoto7/mobicat mobicat
```

## Usage

```sh
# Choose an app installed on the connected devices
mobicat

# Same ID on both platforms
mobicat com.example.app

# Different IDs per platform
mobicat --android com.example.app --ios com.example.App

# Restart the app and capture its startup (on iOS, also print() output)
mobicat com.example.app --launch

# Only crashes, as JSON
mobicat com.example.app --json -q 'is:crash' | jq -r .message

# Read a saved log (logcat text or `log stream` ndjson)
mobicat --file saved.log

# List connected devices and booted simulators
mobicat devices
```

### Keys

| Key | Action |
|---|---|
| `j` `k` / `↓` `↑` | Move (moving up pauses) |
| `Space` | Pause / resume |
| `g` / `G` | Oldest / follow the newest |
| `]c` `[c` | Next / previous crash (`]e` errors, `]w` warnings) |
| `Tab` / `Shift-Tab` | Next / previous target (All, devices) |
| `d` | Target list: All, per OS, per device, change app |
| `/` | Filter query |
| `f` `n` `N` | Search, next, previous |
| `t` | Pick a tag or subsystem |
| `a` | App-only on / off |
| `+` `-` | Raise / lower the minimum level |
| `Enter` | Detail pane |
| `y` `V` `Y` | Copy entry / range / everything shown |
| `Ctrl-s` | Save everything shown to a file |
| `:` | All actions by name |
| `?` | All keys |
| `Esc` / `q` | Close what is open |
| `Ctrl-C` `Ctrl-C` | Quit |

### Filter queries

| Query | Matches |
|---|---|
| `timeout` | Message contains "timeout" |
| `tag:OkHttp` | Tag contains "OkHttp" |
| `-tag:Chatty` | Excludes the tag |
| `tag~:^Net.*` | Regular expression |
| `level:warn` | Warn and above |
| `subsystem:` `category:` | iOS `os_log` fields |
| `os:android` `os:ios` | One platform |
| `is:crash` | Crash blocks |
| `fw:rn` | React Native (`flutter`, `kmp`, `capacitor`, `native`) |
| `age:5m` | The last 5 minutes |

The same key twice means OR, different keys mean AND. `|`, `&` and parentheses are also supported. Matching is case-insensitive unless the query contains an uppercase letter.

## Configuration

mobicat reads `~/.config/mobicat/config.toml` and then the nearest `.mobicat.toml` up from the current directory (to share with your team). Command-line flags win over both. `mobicat config` shows what was loaded.

```toml
app = "com.example.app"

[defaults]
level = "debug"
query = "-tag:Chatty"

[profiles.net]              # keys 1–9 in the order written, or -p net
query = "tag:OkHttp | subsystem:network"

[noise.android]
tags = ["MyNoisyLib"]       # hide more tags
keep = ["Choreographer"]    # show a tag hidden by default
```

## Documentation

- [Requirements](docs/requirements.md): screens, query syntax, keys and settings in detail
- [Architecture](docs/architecture.md)
- [Technical notes](docs/technical-notes.md): how logs are collected on each platform

## License

[Apache-2.0](LICENSE)
