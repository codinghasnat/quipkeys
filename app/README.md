# QuipKeys background app (Windows + macOS)

Runs in the background and corrects each word in whatever app you're typing
in, at the moment you press space.

- **Toggle on/off:** Ctrl+Alt+Q (Windows) · Ctrl+Option+Q (macOS)
- **Undo a correction:** press Backspace straight after it. The word goes
  back to what you typed, and QuipKeys learns to leave that spelling alone.
- **Stays out of:** terminals, code editors and password managers (see the
  list below), and password fields.

Everything runs offline. The dictionary and language model are built into the
binary, so it's one file of about 70 MB that starts in about half a second.

## Build

You need Rust (https://rustup.rs). From this folder:

```
cargo build --release
```

The binary is `target/release/quipkeys` (`quipkeys.exe` on Windows).

## Run

**Windows:** run `target\release\quipkeys.exe`. No admin rights needed. It
can't correct inside apps running as administrator (Windows blocks that)
unless QuipKeys itself runs as administrator.

**macOS:** run `./target/release/quipkeys` from Terminal. The first time, macOS
blocks it. Open System Settings › Privacy & Security, and under both
**Accessibility** and **Input Monitoring** add the app you ran it from
(Terminal, iTerm…). Switch it on, then start QuipKeys again.

To start it at login:
- **Windows:** put a shortcut to the exe in `shell:startup`.
- **macOS:** add it under System Settings › General › Login Items.

## How it works

The platform layer (`windows.rs`: a low-level keyboard and mouse hook;
`macos.rs`: a CGEventTap) turns key presses into simple events. `core.rs`
mirrors what you've typed into the current field, sends each finished word to
the QuipKeys stream decoder, and turns the decoder's edits into "delete N
characters, type this". The space you pressed is held back until the fix is
typed. Keys you press while a fix is being typed are held and replayed in
order afterwards.

Anything that might move the caret makes QuipKeys forget the tracked text and
start fresh. That includes Enter, Tab, arrows, Home/End, clicks, shortcuts
and switching apps. So it never edits text it isn't sure about. It reaches
back at most 6 words, and only for boundary repairs ("mee tyou" → "meet you").

## Excluded apps

To override the default list, put a `quipkeys-exclude.txt` next to the
binary. It takes one executable name per line, in lower case; lines starting
with `#` are comments.

Default list:
- **Windows:** cmd, powershell, Windows Terminal, VS Code, Visual Studio,
  JetBrains IDEs, Sublime, Notepad++, PuTTY, KeePass, 1Password, Bitwarden…
- **macOS:** Terminal, iTerm2, Alacritty, kitty, WezTerm, Warp, Ghostty, VS
  Code, Cursor, Zed, Xcode, JetBrains IDEs, 1Password, Bitwarden, Keychain
  Access…

## Known limits

- It assumes a QWERTY layout for its error model. The characters themselves
  come from your actual layout, so UK and US are both fine.
- Apps that read raw key codes rather than text (some games, remote-desktop
  clients) may not receive the corrections properly. Toggle it off there, or
  add them to the exclude list.
- Autocomplete popups (for example in browsers' address bars) can swallow
  Backspace differently. If a fix looks wrong there, toggle it off.
