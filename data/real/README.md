# Real typing sessions

Exports from the trainer go here (`quipkeys-typing-YYYY-MM-DD.json`).
They're what QuipKeys gets tuned on next: your actual slips, roll-overs and timing,
instead of the simulated typist.

Each file is a JSON array of runs:

```json
[
  {
    "at": "2026-11-03T18:20:11.512Z",   // when
    "mode": "qk",                         // "qk" = QuipKeys on, "raw" = off
    "bs": "on",                           // backspace allowed?
    "words": 30, "secs": 41.2,
    "raw": 74.1,                          // raw WPM (keys typed)
    "eff_qk": 66.8, "eff_raw": 52.3,      // effective WPM with / without QuipKeys
    "acc": 93.3, "fixes": 5, "backspaces": 2,
    "target": "I want to go to the store before it closes tonight. ...",
    "typed":  ["I", "wamt", "to", "gp", ...],     // what you actually typed, per word
    "output": ["I", "want", "to", "go", ...],     // what QuipKeys produced
    "keys": [[0, "I"], [181, " "], [342, "w"], [455, "a"], ...]  // [ms since start, key]
  }
]
```

Only practice sentences are in there, never anything else you type.
