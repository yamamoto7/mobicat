# What stays stable

Scripts, CI jobs and coding agents call mobicat like any other command. This page lists what they can rely on. Until 1.0 these may still change, but every change is listed in the [changelog](../CHANGELOG.md). From 1.0 on, they change only in a major version.

## Flags and commands

- The flags in `mobicat --help` and their meaning.
- The subcommands: `devices`, `config`, `completions`, `man`, `mcp`.
- The MCP tools of `mobicat mcp` and their arguments.
- New flags and subcommands can be added in any version.
- The text of human-readable output (`--plain`, `mobicat devices`, error messages) is for people and can change. Parse `--json` instead.

## `--json` output

One JSON object per line (NDJSON). Every object has `schema`, a number that goes up only on a breaking change. It is `1` today.

| Key | Type | Notes |
|---|---|---|
| `schema` | number | `1` |
| `platform` | string | `android` or `ios` |
| `device` | string | Android serial, simulator UDID or iPhone UDID |
| `app` | string | Package name or bundle ID. Left out when unknown (`--file`) |
| `time` | string | RFC 3339 in the local time zone, microseconds |
| `pid`, `tid` | number | |
| `level` | string | `V` `D` `I` `W` `E` `F` |
| `tag` | string | Logcat tag; on iOS the category, else the last part of the subsystem |
| `subsystem`, `category` | string | iOS only. Left out when empty |
| `library` | string | iOS only: the image that logged the line. Left out when empty |
| `source` | string | `log`, `stdout`, `stderr` (iOS `--launch`), `report` (an OS crash report) or `system` (mobicat's own notes) |
| `app_only` | bool | The line comes from the app's own code, not the OS or frameworks |
| `crash` | bool | Part of a crash. Left out when false |
| `framework` | string | `react-native`, `flutter`, `kmp` or `capacitor`. Left out for native code |
| `message` | string | The whole message; stack traces stay in one entry |

New keys can be added without a schema change, so ignore keys you do not know. A test in `crates/mobicat-core/src/output.rs` fails when the shape changes by accident.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Done: the logs were printed, or the TUI was closed |
| 1 | Error: no device, the app is not installed, a bad query or config |
| 2 | Bad command line (unknown flag, missing value) |

## Config files

`~/.config/mobicat/config.toml` and `.mobicat.toml`:

- `app`, `apps.android`, `apps.ios`
- `defaults.level`, `defaults.query`, `defaults.all`, `defaults.wrap`, `defaults.buffer`
- `profiles.<name>.query`, `profiles.<name>.extends`
- `noise.android.tags`, `noise.android.keep`

Unknown keys are warned about and ignored, so a config written for a newer mobicat still loads in an older one.

## Filter queries

The query syntax in the [README](../README.md#filter-queries) (`tag:`, `level:`, `is:crash`, `age:` and the rest) keeps its meaning. New keys can be added.
