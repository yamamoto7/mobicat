---
name: mobicat
description: Read a mobile app's logs and crashes from Android devices, emulators and iOS simulators with the mobicat CLI. Use when the user's app crashes, shows an error, behaves oddly or needs debugging on a phone, emulator or simulator, or when the user asks to see, check or watch app logs (logcat, Xcode console, os_log, console.log in React Native, print in Flutter).
allowed-tools: Bash(mobicat *)
---

# Reading app logs with mobicat

mobicat prints only the app's own logs (no system noise) from every connected
Android device and booted iOS simulator. A crash is one entry with its whole
stack trace.

Always pass `--last` or `--for` so the command exits. Without them mobicat
keeps following the logs.

## Commands

```sh
mobicat <app-id> --last 5m                  # what happened recently
mobicat <app-id> --last 10m -q is:crash     # only crashes
mobicat <app-id> --last 5m -q level:warn    # warnings and errors
mobicat <app-id> --launch --for 20s         # restart the app, capture its startup
mobicat <app-id> --for 30s                  # follow while something is tried
mobicat devices                             # connected devices and simulators
```

When the Android and iOS IDs differ: `mobicat --android <package> --ios <bundle-id> --last 5m`.

Add `--json` for one JSON object per line (fields include `platform`,
`device`, `level`, `tag`, `subsystem`, `crash`, `message`).

Filters (`-q`) combine: `-q 'os:ios level:error'`, `-q 'tag:OkHttp timeout'`,
`-q '-tag:Chatty'`, `-q 'fw:rn'` (React Native), `-q 'age:2m'`.

## Finding the app ID

Look in the project before asking the user:

- Android: `applicationId` in `android/app/build.gradle(.kts)` or `app/build.gradle(.kts)`
- iOS: `PRODUCT_BUNDLE_IDENTIFIER` in the `.xcodeproj/project.pbxproj`
- Expo: `expo.android.package` and `expo.ios.bundleIdentifier` in `app.json`
- A `.mobicat.toml` with `app = "..."` means the ID can be left out.

## Working well

- Start with `--last 5m -q level:warn` or `-q is:crash`; widen only when needed.
- To reproduce a bug, run `--for 30s` and ask the user to try it, or use
  `--launch --for 20s` for startup problems.
- Quote the crash's first lines and the app frames of the stack trace when
  explaining a fix.
- On iOS, `--last` has Info and above only (the system does not keep Debug
  lines). `print()` output is captured only with `--launch`.

## If mobicat is missing

`command -v mobicat` fails: tell the user to install it
(`brew install yamamoto7/tap/mobicat`, or see
https://github.com/yamamoto7/mobicat#install). Android needs `adb` on PATH;
iOS needs Xcode.
