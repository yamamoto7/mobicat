# Demo

The demo video and the README GIF are recorded from real devices: a small
shopping-list app, **Shelf** (`com.example.shelf`), runs on an Android emulator
and an iOS simulator while mobicat follows both. One script drives the phones
and the terminal from the same timeline, so every take is the same.

| Path | What |
|---|---|
| `android/` | Shelf for Android (Kotlin, no dependencies) |
| `ios/` | Shelf for iOS (SwiftUI; the Xcode project is generated with XcodeGen) |
| `record/timeline.py` | The demo, second by second: taps, keys and captions |
| `record/record.py` | Records the terminal (VHS), the emulator and the simulator |
| `record/compose.py` | Puts them side by side with captions and keys; writes the MP4 and the GIF |

Shelf writes the kind of logs a real app does (network calls, cache and cart
updates, image loads) and crashes on Checkout: an `IllegalStateException` on
Android, a nil force unwrap on iOS.

## Requirements

macOS with Xcode, the Android SDK, [XcodeGen](https://github.com/yonaskolb/XcodeGen),
[VHS](https://github.com/charmbracelet/vhs), tmux, ffmpeg and Python 3 with Pillow.

## Set up the devices

The tabs show device names, so use an emulator named like a phone and a
simulator in English:

```sh
avdmanager create avd -n Pixel_8 -d pixel_8 -k "system-images;android-35;google_apis;arm64-v8a"
emulator -avd Pixel_8 -no-window -no-audio &

UDID=$(xcrun simctl create "iPhone 17 Pro" com.apple.CoreSimulator.SimDeviceType.iPhone-17-Pro com.apple.CoreSimulator.SimRuntime.iOS-26-5)
xcrun simctl boot "$UDID"
xcrun simctl spawn "$UDID" defaults write -g AppleLanguages -array en-US
xcrun simctl shutdown "$UDID" && xcrun simctl boot "$UDID"
```

Only these two should be running while you record; mobicat picks up every
connected device.

## Build and install Shelf

```sh
# Android, plus two look-alikes so the app picker has neighbors
cd demo/android
for id in com.example.notes com.example.weather com.example.shelf; do
  ./gradlew -q assembleDebug -PappId=$id && adb install -r app/build/outputs/apk/debug/app-debug.apk
done

# iOS, plus two look-alikes
cd ../ios && xcodegen generate
xcodebuild -scheme Shelf -sdk iphonesimulator -derivedDataPath build build
xcrun simctl install booted build/Build/Products/Debug-iphonesimulator/Shelf.app
for id in notes fitness; do
  xcodebuild -scheme Shelf -sdk iphonesimulator -derivedDataPath build-$id \
    PRODUCT_BUNDLE_IDENTIFIER=com.example.$id PRODUCT_NAME=$id build
  xcrun simctl install booted build-$id/Build/Products/Debug-iphonesimulator/$id.app
done
```

## Record

```sh
cargo build --release
python3 demo/record/record.py demo/out
python3 demo/record/compose.py demo/out
cp demo/out/mobicat-demo.gif docs/assets/demo.gif
```

`record.py` takes about two minutes. It resets both apps, sets clean status
bars, then plays `timeline.py`. Shelf is driven from outside: deep links on
Android (`shelf://checkout`) and Darwin notifications on iOS
(`notifyutil -p com.example.shelf.checkout`), so the buttons appear pressed in
the recording.

To change the story, edit `timeline.py` and record again.
