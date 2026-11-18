# QuipKeys introduction: script and b-roll plan

About 1 minute 45 seconds. **VO** is what you say. **On screen** says whether it's you
to camera (A-roll), your screen recording (demo) or one of the clips in `out/` (b-roll).
Clip times are the useful part of each clip; trim to taste. Lines in *italics*
are suggested rewrites of your draft. The reasons are in the notes at the end.

---

## 1. Hook (0:00–0:06)

| VO | On screen |
|---|---|
| *"What if every typo fixed itself the moment you made it?"* | **01 slip-and-snap**, full clip (6.5 s). The squiggle, then the snap on space. |

## 2. Grammarly (0:06–0:15)

| VO | On screen |
|---|---|
| "Are you really going to pay for Grammarly in 2026? *Just to click through and accept every change yourself?*" | **16 pay-then-click**, 0–9 s. The click counter climbs, one fix is locked behind an upgrade, and the QuipKeys side stays on 0. |

## 3. Autocorrect sucks (0:15–0:24)

| VO | On screen |
|---|---|
| "Autocorrect already sucks…" | **02 sliding-space**, 0–5 s. Autocorrect's "Hello ice to me you", then QuipKeys slides the spaces. |
| *"Your phone fixed fat-finger typing years ago. Your laptop never did."* | **03 phone-vs-desk**, 0–5 s. |

## 4. Before telepathy (0:24–0:40)

| VO | On screen |
|---|---|
| "There are a few options before we reach telepathy." | **17 before-telepathy**, 0–1.5 s. Title, then the first card. |
| "Wispr Flow and all of its clones. *Great at home. In an open-plan office, everyone hears your emails.*" | 17, 1.5–3 s. The heads turn. |
| "Subvocal. But I don't think anyone wants to see or smell you mouth-breathing in public." | 17, 3–4.5 s. The mouth and the "hhhff". |
| "All of these are still strange today, especially when you're surrounded by other people…" | 17, 4.5–7 s. Telepathy says "Not yet", then the first three get crossed out. |
| "The keyboard isn't strange. It's normal. And there's *so much* we can still improve." | 17, 7–10 s. The keyboard card pops forward. |

## 5. The reveal (0:40–0:46)

| VO | On screen |
|---|---|
| "So I built the brainrot keyboard." | A-roll, to camera. Hard cut. |
| *"It knows where your fingers were, not just which letters you hit."* | **06 where-was-your-finger**, 0–6 s. |

## 6. Just slap the keys (0:46–0:58)

| VO | On screen |
|---|---|
| "It literally lets you slap your fingers on the keyboard, and it fixes your typos like magic." | **05 mash-and-done**, 0–2.7 s (the mashing only, cut before the end card). Then **12 mashed-keys**, 0–6 s. |
| *"For anyone who'd rather talk than stare at their keys."* (finishing your unfinished line) | Demo: you typing fast, eyes on the screen. |

## 7. The proof (0:58–1:06)

| VO | On screen |
|---|---|
| *"Type so sloppily that only one word in six comes out right, and QuipKeys gets three in four right."* | **14 eyes-off-the-keyboard**, full clip (9 s). The numbers are measured. |
| (optional) *"In our simulation, that's about 30% faster."* | **04 the-race**, last 4 s. |

## 8. Smarter than autocorrect (1:06–1:16), optional if the cut is long

| VO | On screen |
|---|---|
| *"It reads the whole sentence. If it isn't sure, it leaves your word alone."* | **08 the-whole-sentence**, 5–9.6 s. "Hasnat" stays. |
| *"And if it picks the wrong word, one shortcut gives you its next guess."* | **11 next-guess**, 2.5–6 s. |

## 9. Everywhere, and private (1:16–1:26)

| VO | On screen |
|---|---|
| *"It works in every app you type in, and stays out of your terminal and your passwords."* | **15 every-app**, 0–7 s. |
| *"It learns the mistakes you make, and none of it leaves your computer."* | **13 it-learns-you**, 0–6 s, then **10 nothing-leaves**, 0–4 s. |

## 10. Dyslexia (1:26–1:38)

| VO | On screen |
|---|---|
| "I had a friend with dyslexia who couldn't type very well and spent a long time thinking about spellings." | A-roll. |
| "But now he doesn't have to think anymore. *He spells it how it sounds, and QuipKeys finds the word.*" | **18 spell-by-sound**, full clip (9 s). These sentences are real engine output. |

## 11. Free, and built together (1:38–1:50)

| VO | On screen |
|---|---|
| "All of this, for free." | **19 build-it-together**, 0–1.3 s. Logo and repo link. |
| *"And the code is public. Want a feature? Open a pull request and we'll review it."* | 19, 1.3–4 s. The idea cards. |
| "It's not perfect yet. Let's build this together!" | 19, 4–8 s. |

## 12. Close (1:50–1:54)

| VO | On screen |
|---|---|
| (silence, or) "QuipKeys. Autocorrect on steroids." | **05 mash-and-done**, 2.75 s to the end (about 4 s). The "Autocorrect on steroids." end card. |

---

## Clips not used

**07 your-rhythm** and **09 undo-and-remember** don't have a slot. 09 can replace
the second half of section 8 if you'd rather show undo than next guess.

## Notes on the wording

- **"Before you even made it"**: it fixes the word when you press space, so
  *"the moment you made it"* is the honest version. "What if…" softens it, so
  it's your call. But commenters will pick at this one.
- **"Open source"**: the licence is PolyForm Noncommercial. People can read,
  change and share the code, but not use it commercially, so it isn't "open
  source" in the usual (OSI) sense, and developers on Twitter *will* say so.
  Either say *"free, and the code is public"* (what the script does now), or
  switch the licence to MIT or Apache-2.0 if you really want "open source". In
  that case anyone could also sell it.
- **"All of this for free"**: true for personal use. The licence footnote is in clip 19.
- **Grammarly** is named only in the voice-over. Clip 16 shows a generic "subscription grammar checker",
  marked as an illustration, so nothing on screen uses their branding.
- **The dyslexia story**: check your friend is happy to be mentioned, even unnamed.
  Spell by sound is an opt-in mode, and it fixes about 3 in 4 sound spellings, not all of them.
- **The numbers** (one in six → three in four, +30%) come from simulated typing in
  RESULTS.md. If anyone asks, say "in our tests".
- It's spelled **Wispr Flow**.
