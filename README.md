# yazi-pick

**English | [日本語](README.ja.md)**

Pick files with [yazi](https://github.com/sxyazi/yazi) in macOS file dialogs: press a hotkey while a file open dialog (NSOpenPanel) is in front, choose a file in yazi running in a floating terminal panel, and the path is typed into the dialog automatically — all the way through confirming "Open".

Think of it as the macOS counterpart of [xdg-desktop-portal-termfilechooser](https://yazi-rs.github.io/docs/tips#file-chooser) on Linux.

## Requirements

- macOS
- [yazi](https://github.com/sxyazi/yazi)
- A terminal: **kitty / Ghostty (>= 1.2.0) / WezTerm / Alacritty**.
  Autodetected in that order; force one with `YAZI_PICK_TERM=wezterm` etc.
- Any hotkey launcher (AeroSpace / skhd / Raycast / Hammerspoon / ...)
- Optional, for the fast native input path: Rust (`cargo`) to build the
  bundled helper

**No window-manager dependency.** All you need is a way to run this script while a file dialog is frontmost.

## Install

yazi-pick comes in two versions — same script, same features. The only
difference is how the picked path is typed into the dialog:

| | Setup | Post-pick input | Extra requirement |
|---|---|---|---|
| **Basic** | copy one shell script | osascript, ~2.5s | none |
| **Fast** | + build the native helper | Accessibility API, ~1.4-1.7s | Rust (`cargo`) |

Start with Basic; drop in the helper anytime and the script picks it up
automatically (see [Speed](#speed)).

### Basic

```sh
mkdir -p ~/.local/bin
curl -fsSL https://raw.githubusercontent.com/WANAB3/yazi-pick/main/yazi-pick -o ~/.local/bin/yazi-pick
chmod +x ~/.local/bin/yazi-pick
```

### Fast — add the native helper

```sh
git clone https://github.com/WANAB3/yazi-pick.git && cd yazi-pick/helper
cargo build --release
cp target/release/yazi-pick-ax ~/.local/bin/
```

Hotkey examples:

**AeroSpace** (`~/.config/aerospace/aerospace.toml`):

```toml
alt-o = 'exec-and-forget ~/.local/bin/yazi-pick'
```

**skhd**:

```
alt - o : ~/.local/bin/yazi-pick
```

On first use, grant your hotkey tool **Accessibility permission**
(System Settings → Privacy & Security → Accessibility). It is needed for
sending keystrokes and driving the dialog via the Accessibility API.

## Usage

1. Open a file dialog (Open / Upload / ...) in any app
2. Trigger yazi-pick with your hotkey
3. Pick a file in yazi and hit Enter
4. The path is filled into the dialog and confirmed

An optional argument sets yazi's starting directory (defaults to `$HOME`):

```sh
yazi-pick ~/Downloads
```

## Speed

With the native helper built, measured on an M4:
**hotkey → picker ~0.5s, pick → dialog confirmed ~1.4-1.7s**.

Each picker is a fresh one-shot terminal instance — nothing stays resident.
The kitty window is styled as a borderless, translucent, centered panel,
Spotlight-style.

- **Native helper (`yazi-pick-ax`)**: drives the dialog through the
  Accessibility C API in-process. The osascript fallback pays 50-150ms of
  Apple Events IPC per UI query; the helper pays microseconds, which cuts
  post-pick latency from ~2.5s to ~1.4-1.7s. Install it next to the script
  (see above); the script picks it up automatically.

## How it works

1. Remembers the app showing the dialog (and which of its windows is key),
   then runs `yazi --chooser-file` in a terminal panel and waits for it to
   close.
2. Types the chosen path into the dialog, with a per-app strategy:
   - **Regular (AppKit) apps**: re-raises the dialog window (macOS may hand
     focus back to a document window instead), opens the Go-to-Folder sheet
     (Cmd+Shift+G) and writes the path via the Accessibility API — but only
     into a *newly focused text field inside an AXSheet*, verifying the write
     by reading it back before confirming. Immune to IME state, multibyte
     paths, and leftover text in the field.
   - **Firefox-family browsers (Zen etc.)**: their file dialogs are remote
     panels that never appear in *any* process's accessibility tree. The
     script proves the dialog exists by diffing the window-server window count
     against the accessibility window count, then drives it with plain
     keystrokes (select-all before paste, so nothing can be concatenated).
   - **No dialog detected**: exits with an error without sending any input.
     Keystrokes are never sprayed into an unsuspecting app.
3. Failures are logged to `$TMPDIR/yazi-pick.log`.

## Troubleshooting

- First stop: `cat "$TMPDIR/yazi-pick.log"`
- `no supported terminal found` → install one of the supported terminals, or
  put it on PATH / in `/Applications`
- Nothing happens → check the Accessibility permission of your hotkey tool

## License

MIT
