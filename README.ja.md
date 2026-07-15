# yazi-pick

**[English](README.md) | 日本語**

macOS のファイル選択ダイアログを [yazi](https://github.com/sxyazi/yazi) で操作するスクリプト。「開く」「ファイルのアップロード」等のダイアログが最前面の状態でホットキーから起動すると、フローティングパネルの yazi が開き、選んだファイルのパスがダイアログに自動入力されて「開く」まで確定される。

Linux の [xdg-desktop-portal-termfilechooser](https://yazi-rs.github.io/docs/tips#file-chooser) の macOS 版に相当。

## 必要なもの

- macOS
- [yazi](https://github.com/sxyazi/yazi)
- ターミナル: **kitty / Ghostty (>= 1.2.0) / WezTerm / Alacritty** のいずれか。
  この順で自動検出する。`YAZI_PICK_TERM=wezterm` のように環境変数で明示指定も可能。
- 起動用のホットキーツール (AeroSpace / skhd / Raycast / Hammerspoon など何でも)
- 任意 (高速なネイティブ入力経路用): 同梱ヘルパのビルドに Rust (`cargo`)

**特定のウィンドウマネージャには依存しない。** 必要なのは「ダイアログが最前面の状態でこのスクリプトを起動する手段」だけ。

## インストール

```sh
mkdir -p ~/.local/bin
curl -fsSL https://raw.githubusercontent.com/WANAB3/yazi-pick/main/yazi-pick -o ~/.local/bin/yazi-pick
chmod +x ~/.local/bin/yazi-pick
```

任意だが推奨 — ネイティブヘルパのビルド ([高速化](#高速化)参照):

```sh
git clone https://github.com/WANAB3/yazi-pick.git && cd yazi-pick/helper
cargo build --release
cp target/release/yazi-pick-ax ~/.local/bin/
```

ホットキーの設定例:

**AeroSpace** (`~/.config/aerospace/aerospace.toml`):

```toml
alt-o = 'exec-and-forget ~/.local/bin/yazi-pick'
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

## 高速化

独立した高速化経路が 2 つ。どちらも任意で、無くても動く。
両方有効 (kitty + ヘルパ) の実測 (M4):
**ホットキー → picker 表示 ~0.2s、選択 → ダイアログ確定 ~1.4-1.7s**。

- **常駐ピッカーインスタンス (kitty のみ・自動)**: 初回実行時にウィンドウレスの
  隠し kitty インスタンスを起動し、以後はソケット越しにピッカーウィンドウを
  生成する (~0.2s。コールド起動は ~0.5-0.7s)。パネルは枠なし・半透明・
  画面中央配置の Spotlight 風で、Dock や Cmd-Tab には現れない。
  不要なら `pkill -f yazi-pickd` でいつでも殺せる (次回起動時に再生成、
  失敗時はワンショットウィンドウにフォールバック)。
- **ネイティブヘルパ (`yazi-pick-ax`)**: ダイアログ操作を Accessibility C API の
  in-process 呼び出しで行う。osascript フォールバックは UI 問い合わせのたびに
  Apple Events IPC (50-150ms) を払うが、ヘルパはマイクロ秒単位。
  選択後の所要が ~2.5s → ~1.4-1.7s になる。スクリプトと同じディレクトリに
  置けば自動で使われる。

## 仕組み

1. ダイアログを出しているアプリ (とその key ウィンドウ) を記録し、ターミナルの
   パネルで `yazi --chooser-file` を実行、閉じるまで待つ
2. 選ばれたパスをダイアログに入力する。アプリによって戦略を変える:
   - **通常 (AppKit アプリ)**: ダイアログウィンドウを再 raise してから
     (macOS はフォーカス返却時にドキュメントウィンドウを key にすることがある)、
     Cmd+Shift+G の「フォルダへ移動」シートを開き、「AXSheet 内の新たに
     フォーカスされたテキスト欄」だけを対象に AX で直接書き込む。書き込み結果を
     読み戻して検証してから確定するので、IME・日本語パス・欄に残った前回パスの
     影響を受けない。
   - **Firefox 系ブラウザ (Zen 等)**: ファイルダイアログがどのプロセスの
     AX ツリーにも現れない (リモートパネル)。ウィンドウサーバから見える
     ウィンドウ数と AX のウィンドウ数の差でダイアログの実在を確認した上で、
     キーイベントだけで入力する (全選択→貼り付けなので連結事故は起きない)。
   - **ダイアログの実在を確認できない場合**: 何も入力せずエラー終了する。
     アプリに勝手にキー入力が流れることはない。
3. 失敗は `$TMPDIR/yazi-pick.log` に記録される

## トラブルシューティング

- 動かないときはまず `cat "$TMPDIR/yazi-pick.log"`
- `no supported terminal found` → 対応ターミナルを入れるか、
  PATH の通った場所か `/Applications` に配置する
- 何も起きない → ホットキーツールのアクセシビリティ権限を確認
- picker の挙動がおかしい → `pkill -f yazi-pickd` してからやり直す

## License

MIT
