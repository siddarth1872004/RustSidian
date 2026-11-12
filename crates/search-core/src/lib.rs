//! Search for Rustsidian: Obsidian's query language, a parallel full-text scan and fuzzy matching.

pub mod engine;
pub mod fuzzy;
pub mod query;

pub use engine::{snippet, Corpus, Hit, Match, NoteView, Query, RankWeights, Sort};
pub use fuzzy::{Fuzzy, FuzzyHit};
