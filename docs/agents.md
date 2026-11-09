# Using mobicat from a coding agent

Coding agents (Claude Code, Codex, Cursor, Gemini CLI and others) run shell
commands and read what they print. mobicat gives them the same logs you see in
the TUI: only your app's, with crashes grouped with their stack traces, from
Android devices and emulators and iOS simulators alike.

Use `--last` or `--for` so the command exits:

```sh
# What happened in the last 5 minutes
mobicat com.example.app --last 5m

# Only crashes
mobicat com.example.app --last 10m -q is:crash

# Restart the app and capture its startup
mobicat com.example.app --launch --for 20s

# Follow while the user (or the agent) tries something
mobicat com.example.app --for 30s
```

Without `--last` or `--for`, mobicat follows the logs until it is stopped,
which an agent waiting for the command to finish does not want.

## Output

The default output has one line per entry:

```
09:57:40.644 A D               OkHttp --> GET https://api.example.com/v1/items
09:58:24.479 i I              network GET https://api.example.com/v1/items
```

The columns are time, platform (`A` Android, `i` iOS), level (`V` `D` `I` `W`
`E` `F`), tag (on iOS, the `os_log` category) and the message. A crash is one
entry with the whole stack trace.

`--json` prints one object per line with every field, including the device,
pid, subsystem, framework and whether the entry is a crash. It is easy to
filter with `jq`:

```sh
mobicat com.example.app --last 10m -q is:crash --json | jq -r .message
```

The same filter queries as in the TUI work with `-q` (see the
[README](../README.md#filter-queries)), for example `-q 'level:warn os:ios'`
or `-q 'tag:OkHttp timeout'`.

## Finding the app ID

mobicat needs the Android package name and the iOS bundle ID. They are usually
in the project:

| Project | Android | iOS |
|---|---|---|
| Native | `applicationId` in `app/build.gradle(.kts)` | `PRODUCT_BUNDLE_IDENTIFIER` in the Xcode project |
| Expo | `expo.android.package` in `app.json` | `expo.ios.bundleIdentifier` in `app.json` |
| React Native | `applicationId` in `android/app/build.gradle` | `PRODUCT_BUNDLE_IDENTIFIER` in `ios/*.xcodeproj` |
| Flutter | `applicationId` in `android/app/build.gradle(.kts)` | `PRODUCT_BUNDLE_IDENTIFIER` in `ios/Runner.xcodeproj` |

When they differ, pass both: `mobicat --android com.example.app --ios com.example.App --last 5m`.
A `.mobicat.toml` with `app = "..."` in the repository saves passing it each time.

`mobicat devices` lists the connected devices and booted simulators.

## Limits

- On iOS, past logs (`--last`) have Info and above. The system does not keep
  Debug lines, so `--for` is the way to capture them.
- iOS `print()` output goes to stdout, not to the system log. It is captured
  only with `--launch`, which restarts the app.
- Physical iOS devices are not supported yet.

## Claude Code plugin

This repository is also a Claude Code plugin marketplace with a skill that
teaches Claude when and how to run mobicat:

```
/plugin install mobicat --marketplace yamamoto7/mobicat
```

On Claude Code before 2.1.275, add the marketplace first:

```
/plugin marketplace add yamamoto7/mobicat
/plugin install mobicat@mobicat
```

The plugin does not install the `mobicat` command itself; see
[Install](../README.md#install).

For other agents, copy the commands above into your `AGENTS.md` or the agent's
instructions.
