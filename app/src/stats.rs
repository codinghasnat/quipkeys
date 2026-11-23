//! How much QuipKeys has helped: counts per day (typos fixed, words checked,
//! texts written...) and an estimate of the time saved. Counts only, never
//! words. Saved to `quipkeys-stats.txt`.
//!
//! Time saved is an estimate, the same model as the speed simulation
//! (src/bin/bench.rs): without QuipKeys, a wrong word costs about half a
//! second to notice, then backspacing over it and retyping it, at the user's
//! own measured typing speed. A "write it for me" text saves the time it
//! would take to type it, minus typing the instruction and waiting for it.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds to notice a wrong word before fixing it by hand.
pub const NOTICE_S: f64 = 0.5;
/// Typing speed assumed until enough of the user's own typing is measured
/// (80 words a minute, 5 characters a word).
pub const DEFAULT_KEYS_PER_S: f64 = 80.0 * 5.0 / 60.0;
/// Characters of typing needed before the measured speed is used.
const SPEED_MIN_KEYS: u64 = 500;
/// A pause longer than this between keys isn't typing time.
pub const TYPING_GAP_MS: f64 = 1500.0;
/// Days kept in the file.
const KEEP_DAYS: u32 = 400;

/// One day's counts.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Day {
    pub words: u64,
    pub fixes: u64,
    pub undos: u64,
    pub guesses: u64,
    pub writes: u64,
    pub written_chars: u64,
    /// Estimated time saved, in milliseconds.
    pub saved_ms: u64,
}

impl Day {
    fn add(&mut self, o: &Day) {
        self.words += o.words;
        self.fixes += o.fixes;
        self.undos += o.undos;
        self.guesses += o.guesses;
        self.writes += o.writes;
        self.written_chars += o.written_chars;
        self.saved_ms += o.saved_ms;
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    /// Local day number (days since 1970-01-01) -> counts.
    pub days: BTreeMap<u32, Day>,
    /// Typing measured so far: keys, and the time they took (gaps under
    /// TYPING_GAP_MS only).
    pub keys: u64,
    pub typing_ms: f64,
    dirty: bool,
}

impl Stats {
    /// The user's typing speed (keys a second), or the default until there's enough.
    pub fn keys_per_s(&self) -> f64 {
        if self.keys < SPEED_MIN_KEYS || self.typing_ms <= 0.0 {
            return DEFAULT_KEYS_PER_S;
        }
        (self.keys as f64 / (self.typing_ms / 1000.0)).clamp(2.0, 15.0)
    }

    /// Words a minute, if measured.
    pub fn wpm(&self) -> Option<f64> {
        (self.keys >= SPEED_MIN_KEYS).then(|| self.keys_per_s() * 60.0 / 5.0)
    }

    fn day(&mut self, today: u32) -> &mut Day {
        self.dirty = true;
        self.days.entry(today).or_default()
    }

    /// The time a fix by hand would have taken (ms): notice, backspace over
    /// what was typed, type what was meant.
    pub fn fix_cost_ms(&self, typed_chars: usize, meant_chars: usize, fixes: usize) -> u64 {
        let s = NOTICE_S * fixes as f64 + (typed_chars + meant_chars) as f64 / self.keys_per_s();
        (s * 1000.0).round() as u64
    }

    pub fn word(&mut self, today: u32) {
        self.day(today).words += 1;
    }

    pub fn key(&mut self, gap_ms: f64) {
        if gap_ms > 0.0 && gap_ms < TYPING_GAP_MS {
            self.keys += 1;
            self.typing_ms += gap_ms;
            self.dirty = true;
        }
    }

    /// QuipKeys fixed `fixes` words; returns the time credited (ms), so an undo can take it back.
    pub fn fixed(&mut self, today: u32, typed_chars: usize, meant_chars: usize, fixes: usize) -> u64 {
        let ms = self.fix_cost_ms(typed_chars, meant_chars, fixes);
        let d = self.day(today);
        d.fixes += fixes as u64;
        d.saved_ms += ms;
        ms
    }

    /// The user undid a fix: it no longer counts.
    pub fn undone(&mut self, today: u32, fixes: usize, ms: u64) {
        let d = self.day(today);
        d.fixes = d.fixes.saturating_sub(fixes as u64);
        d.saved_ms = d.saved_ms.saturating_sub(ms);
        d.undos += 1;
    }

    /// The next-guess key swapped a word: the user noticed it, but didn't
    /// have to retype it.
    pub fn guessed(&mut self, today: u32, from_chars: usize, to_chars: usize) {
        let ms = ((from_chars + to_chars) as f64 / self.keys_per_s() * 1000.0) as u64;
        let d = self.day(today);
        d.guesses += 1;
        d.saved_ms += ms;
    }

    /// "Write it for me" put `written` characters in for an instruction of
    /// `instruction` characters, `waited_ms` after the hotkey.
    pub fn wrote(&mut self, today: u32, written: usize, instruction: usize, waited_ms: u64) {
        let typing = (written as f64 - instruction as f64) / self.keys_per_s() * 1000.0;
        let ms = (typing - waited_ms as f64).max(0.0) as u64;
        let d = self.day(today);
        d.writes += 1;
        d.written_chars += written as u64;
        d.saved_ms += ms;
    }

    /// Totals over days `from..=to`.
    pub fn total(&self, from: u32, to: u32) -> Day {
        let mut t = Day::default();
        for (_, d) in self.days.range(from..=to) {
            t.add(d);
        }
        t
    }

    pub fn all_time(&self) -> Day {
        self.total(0, u32::MAX)
    }

    pub fn to_text(&self) -> String {
        let mut s = String::from("# QuipKeys stats: counts only, no words. Delete this file to reset.\n");
        s.push_str(&format!("typing keys={} ms={:.0}\n", self.keys, self.typing_ms));
        for (day, d) in &self.days {
            s.push_str(&format!(
                "day={day} words={} fixes={} undos={} guesses={} writes={} written={} saved_ms={}\n",
                d.words, d.fixes, d.undos, d.guesses, d.writes, d.written_chars, d.saved_ms
            ));
        }
        s
    }

    pub fn from_text(text: &str) -> Stats {
        let mut st = Stats::default();
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            let kv: BTreeMap<&str, &str> = line.split_whitespace().filter_map(|p| p.split_once('=')).collect();
            let n = |k: &str| kv.get(k).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
            if line.starts_with("typing ") {
                st.keys = n("keys");
                st.typing_ms = kv.get("ms").and_then(|v| v.parse().ok()).unwrap_or(0.0);
            } else if let Some(day) = kv.get("day").and_then(|v| v.parse::<u32>().ok()) {
                st.days.insert(
                    day,
                    Day {
                        words: n("words"),
                        fixes: n("fixes"),
                        undos: n("undos"),
                        guesses: n("guesses"),
                        writes: n("writes"),
                        written_chars: n("written"),
                        saved_ms: n("saved_ms"),
                    },
                );
            }
        }
        st
    }

    fn trim(&mut self, today: u32) {
        let oldest = today.saturating_sub(KEEP_DAYS);
        self.days.retain(|&d, _| d >= oldest);
    }
}

// ------------------------------------------------------------- the app's copy

struct Store {
    stats: Stats,
    path: PathBuf,
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);

/// Seconds east of UTC for local time, set by the platform layer (so "today"
/// is the user's day).
static UTC_OFFSET_S: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

pub fn set_utc_offset(seconds: i64) {
    UTC_OFFSET_S.store(seconds, std::sync::atomic::Ordering::Relaxed);
}

/// Today's local day number.
pub fn today() -> u32 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    ((now + UTC_OFFSET_S.load(std::sync::atomic::Ordering::Relaxed)) / 86_400).max(0) as u32
}

/// Load the stats file and start counting (the app calls this once). Until
/// then, every recording call does nothing (tests, other platforms).
pub fn init(path: PathBuf) {
    let stats = std::fs::read_to_string(&path).map(|t| Stats::from_text(&t)).unwrap_or_default();
    *STORE.lock().unwrap() = Some(Store { stats, path });
    // Save once a minute if anything changed.
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
        save();
    });
}

/// Run `f` on the stats, if counting has started.
pub fn with<R>(f: impl FnOnce(&mut Stats) -> R) -> Option<R> {
    STORE.lock().unwrap().as_mut().map(|s| f(&mut s.stats))
}

pub fn save() {
    let mut g = STORE.lock().unwrap();
    if let Some(s) = g.as_mut() {
        if s.stats.dirty {
            s.stats.trim(today());
            if std::fs::write(&s.path, s.stats.to_text()).is_ok() {
                s.stats.dirty = false;
            }
        }
    }
}

/// Start again from zero (the stats window's Reset button).
pub fn reset() {
    let mut g = STORE.lock().unwrap();
    if let Some(s) = g.as_mut() {
        s.stats = Stats::default();
        let _ = std::fs::remove_file(&s.path);
    }
}

pub const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
pub const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// (year, month 1-12, day 1-31) for a day number (days since 1970-01-01).
pub fn civil(days: u32) -> (i64, u32, u32) {
    // Howard Hinnant's days_from_civil, inverted.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// "about 42 min", "about 3 h 10 min", "12 s".
pub fn duration(ms: u64) -> String {
    let s = ms / 1000;
    if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("about {} min", (s + 30) / 60)
    } else {
        let m = (s + 30) / 60;
        format!("about {} h {} min", m / 60, m % 60)
    }
}

/// 1234 -> "1,234".
pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixes_save_notice_plus_retyping() {
        let mut s = Stats::default();
        // Default 80 WPM = 6.67 keys/s: "teh" -> "the" is 0.5 s + 6 keys = 1.4 s.
        let ms = s.fixed(100, 3, 3, 1);
        assert_eq!(ms, 1400);
        assert_eq!(s.all_time().fixes, 1);
        s.undone(100, 1, ms);
        assert_eq!(s.all_time(), Day { undos: 1, ..Default::default() });
    }

    #[test]
    fn measured_speed_takes_over() {
        let mut s = Stats::default();
        for _ in 0..600 {
            s.key(200.0); // 5 keys a second = 60 WPM
            s.key(5000.0); // a pause: not typing time
        }
        assert_eq!(s.keys, 600);
        assert!((s.wpm().unwrap() - 60.0).abs() < 0.01);
        assert_eq!(s.fix_cost_ms(3, 3, 1), 1700);
    }

    #[test]
    fn writing_saves_typing_minus_waiting() {
        let mut s = Stats::default();
        // 400 characters written for a 40-character instruction, after 2 s:
        // (360 / 6.67) s - 2 s = 52 s.
        s.wrote(100, 400, 40, 2000);
        assert_eq!(s.all_time().saved_ms, 52_000);
        // Never negative.
        s.wrote(100, 10, 40, 5000);
        assert_eq!(s.all_time().saved_ms, 52_000);
        assert_eq!(s.all_time().writes, 2);
    }

    #[test]
    fn days_and_file_round_trip() {
        let mut s = Stats::default();
        s.word(10);
        s.word(11);
        s.fixed(11, 4, 4, 1);
        s.guessed(12, 4, 4);
        s.key(100.0);
        assert_eq!(s.total(11, 12).words, 1);
        assert_eq!(s.total(11, 12).fixes, 1);
        let back = Stats::from_text(&s.to_text());
        assert_eq!(back.days, s.days);
        assert_eq!(back.keys, 1);
        assert!(s.to_text().lines().all(|l| l.starts_with('#') || l.starts_with("day=") || l.starts_with("typing ")));
    }

    #[test]
    fn dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(WEEKDAYS[((0 + 4) % 7) as usize], "Thu");
        // 2026-11-23 is a Monday.
        let d = 20_780;
        assert_eq!(civil(d), (2026, 11, 23));
        assert_eq!(WEEKDAYS[((d + 4) % 7) as usize], "Mon");
        assert_eq!(civil(11_016), (2000, 2, 29));
    }

    #[test]
    fn friendly_numbers() {
        assert_eq!(duration(12_000), "12 s");
        assert_eq!(duration(42 * 60_000), "about 42 min");
        assert_eq!(duration(190 * 60_000), "about 3 h 10 min");
        assert_eq!(thousands(1_234_567), "1,234,567");
        assert_eq!(thousands(12), "12");
    }
}
