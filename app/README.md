# QuipKeys background app (Windows + macOS)

Runs in the background and corrects each word in whatever app you're typing
in, at the moment you press space.

- **Toggle on/off:** Ctrl+Alt+Q (Windows) · Ctrl+Option+Q (macOS)
- **Wrong word? Next guess:** Ctrl+Alt+N (Windows) · Ctrl+Option+N (macOS),
  straight after the word. It swaps in the next most likely reading ("ship"
  → "shop", "of" → "if"); press again for the one after, and a full cycle
  brings back what was there. This is how to fix a slip that made another
  real word, which QuipKeys can't safely change by itself.
- **See what changed:** the tray/menu-bar icon flashes blue for a moment on
  every fix, and **Recent fixes** in its menu lists the last five (kept in
  memory only, gone when QuipKeys quits).
- **Undo a correction:** press Backspace straight after it. The word goes
  back to what you typed, and QuipKeys leaves that spelling alone until it
  quits. To keep it for good, save it in **My words**.
- **Write it for me:** Ctrl+Alt+W (Windows) · Ctrl+Option+W (macOS). Type
  what you want written, copy what it should know (an email), press the key:
  your instruction turns into the finished text. Off until you choose a
  model; see below.
- **Stays out of:** terminals, code editors and password managers (see the
  list below), and password fields.

Everything runs offline. The dictionary and language model are built into the
binary, so it's one file of about 70 MB that starts in about half a second.

It lives in the system tray (Windows) or menu bar (macOS). Click the icon to
pause it, skip the app you're in, turn on start at login, or quit. On first
run a short welcome window explains what it does and how private it is. On
macOS it also walks you through the one permission it needs.

## Private by design

- Typo fixing runs entirely on your computer. It works with no internet
  connection and never connects to the internet.
- Update checks: **Check for updates** in the menu asks GitHub for the latest
  release (a plain request: nothing about you is sent). It only happens when
  you choose it, or once a day if you switch on **Check for updates
  automatically** (off by default).
- The one exception is **Write it for me** with an online model chosen, which
  you have to switch on and give your own API key. Even then, something is
  sent only when you press its hotkey: your instruction and its context, to
  the provider you picked. Choose a model on your computer instead and
  nothing leaves it.
- What you type is held in memory only, for the last few words. It's
  forgotten on Enter, a click or an app switch, and never written to disk.
- The only files it writes are `quipkeys-settings.txt` (whether you've seen
  the welcome screen), `quipkeys-exclude.txt` (apps to skip) and
  `quipkeys-words.txt` (your word list: only words you chose to save), and
  `quipkeys-profile.txt` (tallies of your kinds of slips: counts, no words),
  `quipkeys-stats.txt` (your stats: counts per day, no words), and
  `quipkeys-api-keys.txt` (your API keys, only if you add any). These live
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

## Spell by sound

A free, opt-in mode for people who spell by ear, for example with dyslexia. Write
"fone", "nollij" or "edukashun" and the rest of the sentence settles which word
you meant: "phone", "knowledge", "education". Switch it on in the welcome window
or the menu. With it off, QuipKeys corrects exactly as before. In tests on
spellings it had never seen, it fixed about 3 in 4 in real sentences and wrongly
changed about 1 in 100. Ordinary typo-fixing gets slightly less sharp while it's
on: word accuracy drops by 0.2 to 1.2 points. Details are in the main RESULTS.md.

## It learns your typing

Everyone slips differently: some people swap letters, some drop them, some hit
extra keys. QuipKeys keeps a tally of the kinds of slips *you* make and leans
its corrections that way.
- **What it learns from:** words it fixed that you kept, and ordinary words
  you typed correctly. Undoing a fix takes that example back out.
- **What it stores:** counts only, such as "letters swapped: 35", in
  `quipkeys-profile.txt` on your computer. It never stores a word, and
  nothing is sent anywhere.
- **When it applies:** once it has about 400 words of your typing, and from
  the next time QuipKeys starts.
- **Your control:** **Learn my typing habits** in the menu switches it off;
  **Forget what it learned** wipes it.

It also learns your habitual wrong keys: if you keep hitting M when you mean
N, that slip gets cheaper to fix. A key confusion only counts once it has been
seen at least five times and there are about 2,000 words to go on, and it can
only ever make a fix more likely, never less.

Tested two ways. On one person's recorded typing, learning on one set of runs
and scoring on another raised words right by 0.4 to 1.4 points. In a
simulation of seven kinds of typist over five days, it helped every one (by
up to 1.2 points) and made none worse. Most of the gain came on the first day.

## Write it for me

Turn a quick instruction into finished text, right where you're typing:
an email reply, a Slack message, the next paragraph of a document, a
LinkedIn comment, a support answer.

1. Type what you want: *"agree, but ask if Monday works instead"*.
2. Press **Ctrl+Alt+W** (Windows) or **Ctrl+Option+W** (macOS).
3. The text streams in where your instruction was, a sentence at a time, and
   your clipboard is left as it was.

**Context, without copying.** When you press the key, QuipKeys looks at:
- which app and window you're in, so it knows if it's a chat, an email, a
  document, a social post, a ticket or a form, and writes to fit;
- the text box you're in: what's before and after your cursor (it writes
  only what belongs at the cursor and never repeats what's there);
- the text visible in that window: the conversation, the email you're
  answering, the page you're on;
- anything you copied in the last 15 minutes, if you want to point it at
  something specific.

Press the key with nothing typed and it writes what fits: a reply in a
conversation, otherwise the natural next text.

Set it up under **Write it for me…** in the menu. Pick who writes the text:

- **An online model, with your own API key:** Claude, OpenAI, Gemini, Grok
  or DeepSeek. The window walks you through it:
  1. **Open the provider's key page** (one click) and create a key;
  2. **paste it** and press **Check key**: QuipKeys asks the provider which
     models your key can use;
  3. **pick a model**. It starts on the provider's cheapest one: Claude Haiku
     5.5 for Claude, and for the others the cheapest of a short list
     (`gpt-5-nano`, `gemini-2.5-flash-lite`, `grok-4-1-fast-non-reasoning`,
     `deepseek-v4-flash`…). Providers rename models often, so if none of
     those is offered it picks one whose name says small and fast ("nano",
     "lite", "flash", "mini"). You pay the provider directly for what you use.
- **Custom:** any service with an OpenAI-style chat endpoint (OpenRouter,
  Groq, Mistral, Together…): paste its address and key.
- **On this computer, free:** nothing leaves your computer. QuipKeys checks
  for [Ollama](https://ollama.com) (a free app that runs models locally),
  links to its download page if it isn't there, and downloads a model with
  one click: `llama3.2` (small and quick, about 2 GB) or `gemma3:4b` (better
  writing, about 3.3 GB). LM Studio and other local servers work too: give
  their address.

Keys are stored only on your computer, one per provider, in
`quipkeys-api-keys.txt` (readable only by you on macOS); the usual
environment variables (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`,
`GEMINI_API_KEY`, `XAI_API_KEY`, `DEEPSEEK_API_KEY`) work too. **Use what's on
screen as context** is on by default; switch it off to use only what you
type and copy. **Try it** checks the setup and shows how long the answer
took.

Speed: the answer is streamed and pasted a sentence at a time, the
connection to the model is opened when you save the settings and then
reused, and replies are kept as short as the instruction allows.

How it stays safe:
- Nothing is read or sent until you press the key. Screen text is read
  through the operating system's accessibility features (on macOS, the
  Accessibility permission QuipKeys already has; on Windows, UI Automation)
  and held in memory for that one request.
- Never read: password fields, and the screen text of banking, payment and
  password-manager apps and pages (their app and window name still set the
  tone). Excluded apps can't use the key at all.
- It only deletes your instruction when it knows exactly where it is: typed
  just now, or, if you clicked away, typed right before the cursor (it
  selects it and checks before replacing it). Otherwise the text goes on
  your clipboard and a note tells you to paste it.
- If you keep typing or click while it's writing, it stops pasting; the
  rest waits on your clipboard.
- Copied text that looks like a password, key or link (one "word" of up to
  100 characters) is never used. Everything it reads is treated as material,
  not orders: the model is told to ignore instructions inside it.

Known limits: Enter starts a new instruction (only the text since the last
Enter, click or arrow key counts). Some apps expose little text to
accessibility tools, and Chromium/Electron apps (Chrome, Slack, Teams,
Discord) may only start sharing it from the second press. Apps that block
pasting won't take the text; it stays on the clipboard.

## Your stats

The menu shows a running total ("Fixed 1,234 typos · saved about 42 min"),
and **Your stats…** opens the full picture for today, the last 7 days or all
time:
- **Time saved**, and a chart of it over the last 14 days;
- **Typos fixed** (fixes you undid don't count) and **words checked**;
- **Next guesses used** and texts **written for you**;
- **your typing speed**, measured from your own typing.

Time saved is an estimate, using the same model as the speed simulation in
the main RESULTS.md: without QuipKeys, a wrong word costs about half a second
to notice, then backspacing over it and typing it again, at your measured
speed (80 words a minute until it has measured yours). A Write it for me text
counts the time you'd have spent typing it, minus typing your instruction and
waiting for it. Only counts are stored, never words, and **Reset stats**
starts again from zero.

## Updates

**Check for updates…** in the menu shows your version and asks GitHub whether a
newer one is out. If there is, the item becomes **Install update x.y.z and
restart**: QuipKeys downloads it, checks it against the release's SHA-256
checksums (it won't install anything that doesn't match), puts it in place of
the running copy and restarts. Switch on **Check for updates automatically** to
be told once a day.

- **Windows:** the new `quipkeys.exe` replaces the old one in the same folder
  (the old one is set aside and deleted at the next start). If that folder
  isn't writable, it says so; download from the release page instead.
- **macOS:** the new `QuipKeys.app` replaces the old one. Because the builds
  are ad-hoc signed, macOS treats each version as a new app: switch QuipKeys on
  again in System Settings › Privacy & Security › Accessibility (the welcome
  window shows the way). Signing with an Apple Developer ID would remove this
  step.

**Publishing a release** (maintainers): set `version` in `app/Cargo.toml`,
commit, then push a matching tag: `git tag v0.2.0 && git push origin v0.2.0`.
The Release workflow (`.github/workflows/release.yml`) builds the Windows
program and a universal macOS app (Apple silicon + Intel), and publishes them
with `SHA256SUMS`.

## Your words

QuipKeys never changes a word in your word list, and it fixes typos of those
words ("quipkeyd" → "quipkeys"). **My words…** in the tray/menu opens a window
where you can:
- **Add a word** by typing it in. Use it for names, slang and jargon.
- **Remove a word** with one click.
- **Save a word you undid.** Words you restored with Backspace this session
  are listed at the top, each with a Save button.
- **Search** the list. The newest words are at the top.

Backspace straight after a fix undoes it and protects that word until
QuipKeys quits, but doesn't save it: Backspace is too easy to press by
accident. Changes apply from your next Enter, click or app switch. The list
is a plain text file (`quipkeys-words.txt`), and the window has a link to open
it directly. Only this window writes to the file.

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
