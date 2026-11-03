//! Physical keyboard geometry (US QWERTY, row-staggered).
//!
//! A physical keyboard gives us no touch coordinates, only which key was
//! pressed. Geometry therefore means: how far apart two keys are, which keys
//! are neighbours, and so how plausible it is that a finger aimed at one and
//! landed on the other.

/// Key-unit coordinates (x, y) for every key we model. Row offsets follow a
/// standard ANSI staggered layout.
#[derive(Clone)]
pub struct Keyboard {
    pos: [Option<(f32, f32)>; 128],
}

const ROWS: [(&str, f32); 4] = [
    ("1234567890-=", -0.5),
    ("qwertyuiop[]", 0.0),
    ("asdfghjkl;'", 0.25),
    ("zxcvbnm,./", 0.75),
];

impl Keyboard {
    pub fn qwerty() -> Self {
        let mut pos = [None; 128];
        for (row, (keys, offset)) in ROWS.iter().enumerate() {
            for (i, ch) in keys.chars().enumerate() {
                pos[ch as usize] = Some((i as f32 + offset, row as f32 - 1.0));
            }
        }
        Keyboard { pos }
    }

    pub fn position(&self, c: u8) -> Option<(f32, f32)> {
        self.pos.get(c.to_ascii_lowercase() as usize).copied().flatten()
    }

    /// Euclidean distance in key widths. Unknown keys are "far" (10.0).
    pub fn distance(&self, a: u8, b: u8) -> f32 {
        match (self.position(a), self.position(b)) {
            (Some((ax, ay)), Some((bx, by))) => ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt(),
            _ => 10.0,
        }
    }

    /// Keys whose centres are within ~1.3 key widths: the keys a finger can
    /// plausibly slip onto.
    pub fn adjacent(&self, a: u8, b: u8) -> bool {
        a != b && self.distance(a, b) <= 1.3
    }

    /// All lowercase letter / apostrophe keys adjacent to `c`.
    pub fn neighbours(&self, c: u8) -> Vec<u8> {
        (b'a'..=b'z')
            .chain(std::iter::once(b'\''))
            .filter(|&k| self.adjacent(c, k))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neighbours_make_sense() {
        let k = Keyboard::qwerty();
        assert!(k.adjacent(b'r', b't'));
        assert!(k.adjacent(b'n', b'm'));
        assert!(k.adjacent(b'o', b'p'));
        assert!(k.adjacent(b'r', b'f'));
        assert!(!k.adjacent(b'r', b'p'));
        assert!(k.distance(b'q', b'p') > 8.0);
        let mut n = k.neighbours(b's');
        n.sort();
        assert_eq!(n, b"adewxz".to_vec());
    }
}
