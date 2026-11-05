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

It lives in the system tray (Windows) or menu bar (macOS). Click the icon to
pause it, skip the app you're in, turn on start at login, or quit. On first
run a short welcome window explains what it does and how private it is. On
macOS it also walks you through the one permission it needs.

## Private by design

- Everything runs on your computer. It works with no internet connection, and
  QuipKeys never connects to the internet.
- What you type is held in memory only, for the last few words. It's
  forgotten on Enter, a click or an app switch, and never written to disk.
- The only files it writes are `quipkeys-settings.txt` (whether you've seen
  the welcome screen), `quipkeys-exclude.txt` (apps to skip) and
  `quipkeys-words.txt` (your word list: only words you taught it). These live
  next to `quipkeys.exe` on Windows (or in `%APPDATA%\QuipKeys` if that folder
  isn't writable), and in `~/Library/Application Support/QuipKeys` on macOS.

## Build and run: Windows

You need Rust (https://rustup.rs). In this folder:

```
cargo build --release
target\release\quipkeys.exe
```

The release build has no console window; look for the blue Q in the tray. It
needs the WebView2 runtime for the welcome window, which is built into
Windows 10 (recent updates) and 11. It can't correct inside apps running as
administrator unless QuipKeys itself does.

## Build and run: macOS

```
sh macos/make-app.sh
open dist/QuipKeys.app
```

That builds `dist/QuipKeys.app`, a menu-bar app with no Dock icon. Drag it
into Applications. The welcome window asks for **Accessibility** permission
(System Settings › Privacy & Security › Accessibility). QuipKeys starts working
the moment it's switched on.

**Updating:** `git pull`, then `sh macos/make-app.sh --install`. That quits
the running copy, replaces `/Applications/QuipKeys.app` and starts the new
one. Each ad-hoc-signed build looks like a new app to macOS, so it clears the
old permission entry. Switch QuipKeys on again when the welcome window asks.

**Debugging:** run `QK_DEBUG=1 /Applications/QuipKeys.app/Contents/MacOS/quipkeys`
from Terminal (quit the menu-bar copy first). It prints every decision. If
corrections leave stray letters ("iI"), raise the pauses between injected
keys with `QK_BS_DELAY_MS`, `QK_SETTLE_MS` and `QK_KEY_DELAY_MS` (defaults 3,
15 and 2 ms).

**Start at login** (tray/menu option, or the checkbox at the end of the welcome
window) uses the registry `Run` key on Windows and a LaunchAgent in
`~/Library/LaunchAgents` on macOS.

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

## Your words

QuipKeys never changes a word in your word list, and it fixes typos of those
words ("quipkeyd" → "quipkeys"). Words get there in two ways:
- **Backspace straight after a fix.** The fix is undone and the word you
  typed is added.
- **By hand.** **My words…** in the tray/menu opens the list. Add or remove
  words, one per line; changes apply from the next Enter or click.

Typing the same unknown word twice also teaches it, but only until QuipKeys
quits. That's deliberate: saving it would mean writing what you type to disk.

## Excluded apps

**Don't correct in …** in the tray/menu skips the app you last typed in.
**Edit excluded apps…** opens the full list. It takes one program name per
line, in lower case.

Default list:
- **Windows:** cmd, powershell, Windows Terminal, VS Code, Visual Studio,
  JetBrains IDEs, Sublime, Notepad++, PuTTY, KeePass, 1Password, Bitwarden…
- **macOS:** Terminal, iTerm2, Alacritty, kitty, WezTerm, Warp, Ghostty, VS
  Code, Cursor, Zed, Xcode, JetBrains IDEs, 1Password, Bitwarden, Keychain
  Access…

Password fields are always skipped: on macOS automatically, on Windows for
standard password boxes.

## Known limits

- It assumes a QWERTY layout for its error model. The characters themselves
  come from your actual layout, so UK and US are both fine.
- Apps that read raw key codes rather than text (some games, remote-desktop
  clients) may not receive the corrections properly. Toggle it off there, or
  add them to the exclude list.
- Autocomplete popups (for example in browsers' address bars) can swallow
  Backspace differently. If a fix looks wrong there, toggle it off.
