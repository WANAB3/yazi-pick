# yazi-pick

Pick files with [yazi](https://github.com/sxyazi/yazi) in macOS file dialogs: press a hotkey while a file open dialog (NSOpenPanel) is in front, choose a file in yazi running in a new terminal window, and the path is typed into the dialog automatically.

macOS のファイル選択ダイアログを yazi で操作するスクリプト。「開く」「ファイルのアップロード」等のダイアログが最前面の状態でホットキーから起動すると、ターミナルの新規ウィンドウに yazi が開き、選んだファイルのパスがダイアログに自動入力されて「開く」まで確定される。

## 必要なもの

- macOS
- [yazi](https://github.com/sxyazi/yazi)
- ターミナル: **kitty / Ghostty (>= 1.2.0) / WezTerm / Alacritty** のいずれか。
  この順で自動検出する。`YAZI_PICK_TERM=wezterm` のように環境変数で明示指定も可能。
- 起動用のホットキーツール (AeroSpace / skhd / Raycast / Hammerspoon など何でも)

**特定のウィンドウマネージャには依存しない。** 必要なのは「ダイアログが最前面の状態でこのスクリプトを起動する手段」だけ。

## インストール

```sh
mkdir -p ~/.local/bin
curl -fsSL https://raw.githubusercontent.com/WANAB3/yazi-pick/main/yazi-pick -o ~/.local/bin/yazi-pick
chmod +x ~/.local/bin/yazi-pick
```

ホットキーの設定例:

**AeroSpace** (`~/.config/aerospace/aerospace.toml`):

```toml
alt-o = 'exec-and-forget ~/.local/bin/yazi-pick'

# picker のウィンドウをフローティングで出す (任意)
[[on-window-detected]]
if.window-title-regex-substring = 'yazi-picker'
run = 'layout floating'
```

**skhd**:

```
alt - o : ~/.local/bin/yazi-pick
```

初回実行時、ホットキーツールに**アクセシビリティ権限**が必要
(システム設定 → プライバシーとセキュリティ → アクセシビリティ)。
キーストローク送信と AX 操作に使う。

## 使い方

1. アプリのファイル選択ダイアログを開く
2. ホットキーで yazi-pick を起動
3. yazi でファイルを選んで Enter
4. パスがダイアログに自動入力される

引数で yazi の開始ディレクトリを指定できる (省略時は `$HOME`):

```sh
yazi-pick ~/Downloads
```

## 仕組み

1. ダイアログを出しているアプリを記録し、ターミナルの新規ウィンドウで
   `yazi --chooser-file` を実行、閉じるまで待つ
2. 選ばれたパスをダイアログに入力する。アプリによって戦略を変える:
   - **通常 (AppKit アプリ)**: Cmd+Shift+G の「フォルダへ移動」シートを開き、
     「AXSheet 内の新たにフォーカスされたテキスト欄」だけを対象に
     アクセシビリティ API で直接書き込む。書き込み結果を読み戻して検証してから
     確定するので、IME・日本語パス・欄に残った前回パスの影響を受けない。
   - **Firefox 系ブラウザ (Zen 等)**: ファイルダイアログがどのプロセスの
     AX ツリーにも現れない (リモートパネル)。ウィンドウサーバから見える
     ウィンドウ数と AX のウィンドウ数の差でダイアログの実在を確認した上で、
     キーイベントだけで入力する (全選択→貼り付けなので連結事故は起きない)。
   - **ダイアログの実在を確認できない場合**: 何も入力せずエラー終了する。
     アプリに勝手にキー入力が流れることはない。
3. 失敗は `$TMPDIR/yazi-pick.log` に記録される

## トラブルシューティング

- 動かないときはまず `cat "$TMPDIR/yazi-pick.log"`
- `対応ターミナルが見つかりません` → 対応ターミナルを入れるか、
  PATH の通った場所か `/Applications` に配置する
- 何も起きない → ホットキーツールのアクセシビリティ権限を確認

## License

MIT
