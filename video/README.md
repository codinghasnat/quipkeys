# Pitch-video animations

Nineteen short clips to cut between demo footage. Each is 1920×1080, 30 fps, H.264,
with no audio. They use the same look as the website: yellow paper, keycaps,
red squiggles for typos and blue for fixes. The finished files are in `out/`.

| Clip | Length | What it shows | The difference from laptop autocorrect |
|---|---|---|---|
| `01-slip-and-snap` | 6.5 s | A finger lands between N and M; "I wamt" gets a red squiggle; the space bar slams and it snaps to "want" | "Laptop autocorrect: underlines it. That's it." vs "QuipKeys: fixed the moment you hit space." |
| `02-sliding-space` | 7 s | "Hellon ice to mee tyou": autocorrect's word-by-word guess vs QuipKeys sliding the spaces | Autocorrect gives "Hello ice to me you"; QuipKeys gives "Hello nice to meet you" |
| `03-phone-vs-desk` | 7.5 s | Phone fixes fat-finger taps, the laptop types exactly what you hit, then the QuipKeys keycap drops in | "Your phone fixed this years ago. Your keyboard never did." / "Until now." |
| `04-the-race` | 11.5 s | Two identical sloppy typists: one stops to backspace, one keeps going; 56 vs 72 wpm, +30% | Backspace counter climbs on the autocorrect side; footnote explains the simulation |
| `05-mash-and-done` | 7 s | Keyboard mashing with the screen shaking, every typo snapping blue, then a hard cut to "Autocorrect on steroids." + logo + URL | The closer |
| `06-where-was-your-finger` | 7.5 s | The typed M lights up, halos show the finger was probably on N, candidate words compete, "want" wins | "QuipKeys knows where the keys are. Laptop autocorrect only compares letters." |
| `07-your-rhythm` | 8.8 s | Keystrokes on a timeline: S and I 22 ms apart, so they were swapped ("thsi" → "this"); a long pause before M ("wamt" → "want") | "Your typing rhythm is evidence. Laptop autocorrect never sees it." |
| `08-the-whole-sentence` | 9.6 s | "I'll meet you form the station": confidence passes 90%, so "from"; "Lunch with Hasnat": 41%, so it's left alone | Autocorrect ignores "form" (a real word) and turns "Hasnat" into "Hasn't" |
| `09-undo-and-remember` | 8.5 s | "that fit is bussin" gets fixed, one Backspace undoes it, and the word flies into "My words"; next time it stays | "Laptop autocorrect keeps 'fixing' it." |
| `10-nothing-leaves` | 9.6 s | Corrections keep working with the Wi-Fi cut and nothing reaching the cloud; then Spell by sound: fone → phone, nollij → knowledge | Private and offline; free for dyslexic typists too |
| `11-next-guess` | 8.5 s | "go to the ship": a real word, so nothing flags it. Ctrl+Alt+N swaps in "shop"; again on "let me know of you can" → "if". The menu-bar icon flashes blue on each change | Slips that make another real word get past every autocorrect; here one key fixes them |
| `12-mashed-keys` | 9 s | "jrkko how are you": four of five keys are one off. The keys you hit go red, the ones you meant get a blue ring, and it becomes "hello" | "Laptop autocorrect: no suggestions." |
| `13-it-learns-you` | 9.5 s | Day 1 to day 5: kept fixes tick past, a heat map builds on the keys (an illustrative M-for-N typist), and the profile file fills with counts. "no words · no sentences · nothing sent" | "It learns how you slip. Not what you write." |
| `14-eyes-off-the-keyboard` | 9 s | Bar chart of words right: typing blind 61.7% → 88.9%; mashing keys 17.2% → 74.7% | Measured on simulated typing, 1,500 held-out sentences per level (RESULTS.md) |
| `15-every-app` | 9.5 s | Generic chat, email, document and notes windows each fix a typo; a terminal ("git psuh") and a password field are left alone | "Every app you type in. Except the ones you shouldn't." |
| `16-pay-then-click` | 9.5 s | A generic subscription grammar checker (labelled as an illustration): a cursor clicks Accept on each fix, one fix is "Premium only", and the click counter climbs. QuipKeys fixes the same text as it's typed with 0 clicks | "Pay every month. Then click every fix yourself." |
| `17-before-telepathy` | 10 s | Cards for voice dictation (heads turn), silent speech (a breathing mouth) and telepathy ("Not yet") get crossed out; the keyboard card pops forward | "The keyboard isn't weird. It's just never been fixed." |
| `18-spell-by-sound` | 9 s | "I need to call my frend on the fone", "the docter sed I am fine", "my skool has a new libary" each get fixed as typed. All are real engine output with Spell by sound on | Footnote: fixes about 3 in 4 sound spellings in real sentences, and misses some |
| `19-build-it-together` | 8 s | Logo, repo link, and idea cards that anyone can pick up (layouts, languages, Linux, "Your idea here") | "Free. The code's public. Not perfect yet. Let's build it together." Licence footnote |

Suggested five for a short cut: 01, 02, 03, 04, 05. For a value-focused cut: 03, 12, 14, 11, 05. The full intro video's script, with a clip for each line, is in [SCRIPT.md](SCRIPT.md).

## Re-rendering

Each scene is an HTML page in `scenes/` whose `render(t)` draws the frame at
time `t`. It follows Remotion's rule for code-driven video: everything is
computed from the frame time with eased interpolation, and nothing uses CSS
animations or timers. That way every frame renders the same way each time.
`render.js` opens each page in headless Chromium, captures every frame and
pipes the frames into ffmpeg.

```
node video/render.js            # all scenes -> video/out/*.mp4
node video/render.js 03 05      # just these
node video/render.js --stills 07  # four still frames per scene, for checking
```

This needs Node with Playwright, and ffmpeg. To change wording, timing or
colours, edit the scene's HTML; shared colours and fonts are in `style.css`,
and easing and keyboard helpers are in `lib.js`.
