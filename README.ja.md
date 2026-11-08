# mobicat

[![CI](https://github.com/yamamoto7/mobicat/actions/workflows/ci.yml/badge.svg)](https://github.com/yamamoto7/mobicat/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

**Android と iOS のアプリのログを、1つのターミナル画面で。**

Android の端末とエミュレータ、iOS シミュレータのログを時刻順に1本にまとめ、自分のアプリのログだけを表示します。アプリを再起動しても追い続け、クラッシュは目立つように出します。

[English](README.md)

![Android エミュレータと iOS シミュレータのログを mobicat で並べて見ている様子。クラッシュが赤く目立っている](docs/assets/demo.gif)

## なぜ mobicat か

Android と iOS の両方に出すアプリ（React Native、Flutter、Kotlin Multiplatform、Capacitor、または2つのネイティブアプリ）では、ログが2か所に分かれます。Android Studio の Logcat と、Xcode のコンソールです。どちらも片方の OS しか映さず、OS が出すノイズも多く混ざります。

mobicat は、両方を1つのターミナルにまとめます。

|  | mobicat | Android Studio の Logcat | Xcode のコンソール | pidcat |
|---|:-:|:-:|:-:|:-:|
| Android | ✓ | ✓ | | ✓ |
| iOS シミュレータ | ✓ | | ✓ | |
| Android と iOS を1画面で | ✓ | | | |
| ターミナルで動く（SSH 越しでも） | ✓ | | | ✓ |
| スクリプトや AI エージェント向けの JSON 出力 | ✓ | | | |

## 特徴

- **両方の OS を1画面に**：つながっているすべての Android 端末と起動中のシミュレータのログを、1本の流れで表示します。`Tab` で「All」「OS ごと」「端末ごと」を切り替えられます。
- **自分のアプリだけ**：Android はアプリの UID、iOS は実行ファイルでログを取ります。OS が出すノイズは最初から隠します（`a` で全部表示）。再起動しても見失わず、`── process started (pid N) ──` の行で区切ります。
- **クラッシュが目立つ**：Java / Kotlin の例外、ネイティブのクラッシュ、Swift の Fatal error、キャッチされなかった例外を、スタックトレースごと赤い `✖ CRASH` の塊にまとめます。見ていない端末で起きたクラッシュは、その端末のタブに印が付きます。
- **Android Studio と同じ絞り込みの書き方**：`tag:OkHttp level:warn -tag:Chatty` に加えて、`subsystem:`、`os:`、`fw:rn`、`age:5m` なども使えます。打つたびに結果が変わり、補完と履歴もあります。
- **クロスプラットフォームのフレームワーク**：React Native、Flutter、Kotlin Multiplatform、Capacitor のログを見分けます（クエリの `fw:`）。
- **スクリプトから使える**：`--json` で NDJSON、`--plain` で1行ずつ出力します。`jq` や CI、AI エージェントから使えます。

## インストール

> まだ最初のリリースを出していません。それまでは、ソースからビルドしてください（いちばん下の方法）。

```sh
# Homebrew（macOS、Linux）
brew trust --tap yamamoto7/tap   # Homebrew 6 以降は、先に tap を信頼する必要があります
brew install yamamoto7/tap/mobicat

# シェルのインストーラ（macOS、Linux）
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/yamamoto7/mobicat/releases/latest/download/mobicat-installer.sh | sh

# PowerShell のインストーラ（Windows）
powershell -ExecutionPolicy Bypass -c "irm https://github.com/yamamoto7/mobicat/releases/latest/download/mobicat-installer.ps1 | iex"

# ソースから（Rust 1.88 以降）
cargo install --locked --git https://github.com/yamamoto7/mobicat mobicat
```

あわせて次のものが必要です。

- Android：`adb`（Android SDK の platform-tools）に `PATH` が通っていること
- iOS：macOS と Xcode（`xcrun simctl`）。今はシミュレータだけで、iOS の実機にはまだ対応していません。

## 使い方

```sh
# つながっている端末に入っているアプリから選ぶ
mobicat

# 両方の OS で同じ ID のとき
mobicat com.example.app

# OS ごとに ID が違うとき
mobicat --android com.example.app --ios com.example.App

# アプリを起動し直して、起動直後のログから取る（iOS は print() の出力も取る）
mobicat com.example.app --launch

# クラッシュだけを JSON で
mobicat com.example.app --json -q 'is:crash' | jq -r .message

# 保存したログを読む（logcat のテキストか log stream の ndjson）
mobicat --file saved.log

# つながっている端末と起動中のシミュレータを一覧する
mobicat devices
```

### キー

| キー | 働き |
|---|---|
| `j` `k` / `↓` `↑` | 移動（上へ動かすと一時停止） |
| `Space` | 一時停止／再開 |
| `g` / `G` | いちばん古い行へ／最新を追う |
| `]c` `[c` | 次／前のクラッシュ（`]e` はエラー、`]w` は警告） |
| `Tab` / `Shift-Tab` | 次／前の対象（All、端末） |
| `d` | 対象の一覧：All、OS ごと、端末ごと、アプリの切り替え |
| `/` | 絞り込みのクエリ |
| `f` `n` `N` | 検索、次、前 |
| `t` | tag や subsystem を選んで絞る |
| `a` | app-only のオン／オフ |
| `+` `-` | 最低レベルを上げる／下げる |
| `Enter` | 詳細ペイン |
| `y` `V` `Y` | 1行／範囲／表示中の全部をコピー |
| `Ctrl-s` | 表示中の全部をファイルに保存 |
| `:` | 操作を名前で探して実行 |
| `?` | すべてのキー |
| `Esc` / `q` | 開いているものを閉じる |
| `Ctrl-C` `Ctrl-C` | 終了 |

### 絞り込みのクエリ

| クエリ | 一致するもの |
|---|---|
| `timeout` | メッセージに「timeout」を含む |
| `tag:OkHttp` | tag に「OkHttp」を含む |
| `-tag:Chatty` | その tag を除く |
| `tag~:^Net.*` | 正規表現 |
| `level:warn` | warn 以上 |
| `subsystem:` `category:` | iOS の `os_log` の項目 |
| `os:android` `os:ios` | 片方の OS |
| `is:crash` | クラッシュの塊 |
| `fw:rn` | React Native（ほかに `flutter`、`kmp`、`capacitor`、`native`） |
| `age:5m` | 直近5分 |

同じキーが並ぶと OR、違うキーは AND になります。`|`、`&`、カッコも使えます。クエリに大文字が含まれるときだけ、大文字と小文字を区別します。

## 設定

`~/.config/mobicat/config.toml` を読み、次に今いるディレクトリから上へたどって最初に見つかった `.mobicat.toml`（チームで共有する用）を読みます。コマンドのフラグはどちらよりも優先されます。読んだ内容は `mobicat config` で確かめられます。

```toml
app = "com.example.app"

[defaults]
level = "debug"
query = "-tag:Chatty"

[profiles.net]              # 書いた順に 1〜9 キー、または -p net
query = "tag:OkHttp | subsystem:network"

[noise.android]
tags = ["MyNoisyLib"]       # 隠す tag を足す
keep = ["Choreographer"]    # 既定で隠す tag を表示に戻す
```

## しくみ

- **Android**：端末ごとに `adb logcat --uid <アプリの UID>` を使います。再起動で PID が変わっても UID は変わらないので、アプリのプロセスの行を追い続けられます。どのアプリでも出るフレームワークの tag は隠し、アプリ自身の tag や同梱の SDK のログは残します。
- **iOS シミュレータ**：`xcrun simctl spawn <udid> log stream --style ndjson` を、アプリの実行ファイルで絞って使います。`.app` の中のコードが出した行、subsystem が bundle ID で始まる行、React Native の行、Swift の実行時ライブラリの行を、アプリ自身のログとして扱います。
- 複数行のメッセージ（スタックトレース）は1件にまとめ、OS ごとのクラッシュの形に印を付けます。

詳しくは、[requirements](docs/requirements.md)、[architecture](docs/architecture.md)、[technical notes](docs/technical-notes.md)（いずれも英語）を見てください。

## 今後の予定

- iOS の実機への対応
- 最初のリリースで、ビルド済みのバイナリと Homebrew の formula を出す

アイデアや不具合の報告は [issues](https://github.com/yamamoto7/mobicat/issues) へどうぞ。日本語でもかまいません。

## 開発に参加する

[CONTRIBUTING.md](CONTRIBUTING.md) を見てください（英語）。上のデモは、[`demo/`](demo/README.md) のサンプルアプリとスクリプトで撮っています。

## 謝辞

mobicat は「両方の OS で使える [pidcat](https://github.com/JakeWharton/pidcat)」を目指して作り始めました。絞り込みの書き方は Android Studio の Logcat にならっています。

## ライセンス

[Apache-2.0](LICENSE)
