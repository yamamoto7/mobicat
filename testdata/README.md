# Test fixtures

| File | Origin |
|---|---|
| `android/logcat-epoch-uid.txt` | Recorded from an Android 15 (API 35) emulator with `adb logcat --uid=<uid> -v threadtime -v uid -v epoch -v usec -b main,system,crash`. Two processes (pid 3727, then 3990 after a restart). |
| `android/logcat-threadtime.txt` | Recorded with the default `adb logcat -v threadtime`. |
| `android/logcat-threadtime-uid.txt` | Recorded with `adb logcat -v threadtime -v uid` (note the non-numeric uid `wifi`). |
| `ios/sim-log-stream.ndjson` | Recorded from an iOS 26.5 simulator with `xcrun simctl spawn <udid> log stream --style ndjson --level debug --predicate 'process == "<exe>"'`. Two processes (pid 80225, then 80248). |
| `ios/sim-handmade.ndjson` | Hand-written in the recorded format, to cover the app's own logs, React Native, Kermit, a Swift fatal error, and a different app with the same executable name. |

Recorded files were sanitized: the app ID is replaced with `com.example.app`, the executable with `ExampleApp`, and the user name in paths with `dev`.
