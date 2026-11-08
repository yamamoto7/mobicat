# mobicat

Android と iOS のアプリのログを、1つのターミナル画面で見るツールです。

Android の端末とエミュレータ（`adb logcat`）、iOS シミュレータ（`simctl log stream`）のログを時刻順に1本にまとめ、自分のアプリのログだけを表示します。アプリを再起動しても追い続け、クラッシュは目立つように出します。

[English](README.md)

![Android エミュレータと iOS シミュレータのログを mobicat で並べて見ている様子。クラッシュが赤く目立っている](docs/assets/demo.gif)

## 特徴

- **両方の OS を1画面に**：つながっているすべての Android 端末と起動中のシミュレータのログを、1本の流れで表示します。`Tab` で「All」「OS ごと」「端末ごと」を切り替えられます。
- **自分のアプリだけ**：Android はアプリの UID、iOS は実行ファイルでログを取ります。OS が出すノイズは最初から隠します（`a` で全部表示）。再起動しても見失わず、`── process started (pid N) ──` の行で区切ります。
- **クラッシュが目立つ**：Java / Kotlin の例外、ネイティブのクラッシュ、Swift の Fatal error、キャッチされなかった例外を、スタックトレースごと赤い `✖ CRASH` の塊にまとめます。見ていない端末で起きたクラッシュは、その端末のタブに印が付きます。
- **Android Studio と同じ絞り込みの書き方**：`tag:OkHttp level:warn -tag:Chatty` に加えて、`subsystem:`、`os:`、`fw:rn`、`age:5m` なども使えます。打つたびに結果が変わり、補完と履歴もあります。
- **クロスプラットフォームのフレームワーク**：React Native、Flutter、Kotlin Multiplatform、Capacitor のログを見分けます（クエリの `fw:`）。
- **スクリプトから使える**：`--json` で NDJSON、`--plain` で1行ずつ出力します。`jq` や CI、AI エージェントから使えます。

## 必要なもの

- Android：`adb`（Android SDK の platform-tools）に `PATH` が通っていること
- iOS：macOS と Xcode（`xcrun simctl`）。今はシミュレータだけで、iOS の実機にはまだ対応していません。

## インストール

ビルド済みのバイナリはまだ公開していません。Rust 1.88 以降でソースからビルドします。

```sh
cargo install --locked --git https://github.com/yamamoto7/mobicat mobicat
```

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

## 資料（英語）

- [Requirements](docs/requirements.md)：画面、クエリの書き方、キー、設定の詳細
- [Architecture](docs/architecture.md)
- [Technical notes](docs/technical-notes.md)：OS ごとのログの取り方

## ライセンス

[Apache-2.0](LICENSE)
