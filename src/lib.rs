//! QuipKeys: an offline probabilistic typing decoder.
//!
//! Pipeline: raw keys (+ timing) -> geometry-weighted trie search
//! ([`search`]) -> noisy-channel + context scoring and confidence gate
//! ([`decoder`]) -> streaming application with hysteresis ([`stream`]).

pub mod channel;
pub mod decoder;
pub mod keyboard;
pub mod lexicon;
pub mod ngrams;
pub mod search;
pub mod sim;
pub mod stream;

pub use decoder::{Config, Ctx, Engine, Suggestion};
pub use stream::{Edit, Stream};
pub use decoder::Hyp;
