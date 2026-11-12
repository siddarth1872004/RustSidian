//! Fuzzy matching for the quick switcher and command palette.

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use rayon::prelude::*;

const PARALLEL_THRESHOLD: usize = 8192;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FuzzyHit {
    /// Index into the candidate list.
    pub index: usize,
    pub score: u32,
    /// Char indices of the matched characters, for highlighting.
    pub positions: Vec<u32>,
}

/// Reusable matcher; keeps its scratch buffers between keystrokes.
pub struct Fuzzy {
    matcher: Matcher,
    buf: Vec<char>,
}

impl Default for Fuzzy {
    fn default() -> Self {
        Fuzzy::new()
    }
}

impl Fuzzy {
    pub fn new() -> Fuzzy {
        Fuzzy { matcher: Matcher::new(Config::DEFAULT.match_paths()), buf: Vec::new() }
    }

    /// Best `limit` candidates for `query`, best first. An empty query returns the first
    /// `limit` candidates in their given order (callers pass recents first).
    pub fn top(&mut self, query: &str, candidates: &[&str], limit: usize) -> Vec<FuzzyHit> {
        if query.trim().is_empty() {
            return (0..candidates.len().min(limit))
                .map(|index| FuzzyHit { index, score: 0, positions: Vec::new() })
                .collect();
        }
        let pattern = Pattern::new(query, CaseMatching::Smart, Normalization::Smart, AtomKind::Fuzzy);
        let mut scored: Vec<(u32, usize)> = if candidates.len() >= PARALLEL_THRESHOLD {
            // Large lists are scored in parallel, each worker with its own matcher.
            candidates
                .par_chunks(4096)
                .enumerate()
                .map_init(
                    || (Matcher::new(Config::DEFAULT.match_paths()), Vec::new()),
                    |(matcher, buf), (chunk_i, chunk)| {
                        let mut out = Vec::new();
                        for (i, c) in chunk.iter().enumerate() {
                            if let Some(score) = pattern.score(Utf32Str::new(c, buf), matcher) {
                                out.push((score, chunk_i * 4096 + i));
                            }
                        }
                        out
                    },
                )
                .flatten()
                .collect()
        } else {
            let mut out = Vec::new();
            for (i, c) in candidates.iter().enumerate() {
                if let Some(score) = pattern.score(Utf32Str::new(c, &mut self.buf), &mut self.matcher) {
                    out.push((score, i));
                }
            }
            out
        };
        let n = limit.min(scored.len());
        if n == 0 {
            return Vec::new();
        }
        // Partial selection keeps this O(candidates) instead of sorting everything.
        if n < scored.len() {
            scored.select_nth_unstable_by(n - 1, |a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            scored.truncate(n);
        }
        scored.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.into_iter().map(|(score, index)| FuzzyHit { index, score, positions: Vec::new() }).collect()
    }

    /// Matched character positions for one candidate (computed only for displayed rows).
    pub fn positions(&mut self, query: &str, candidate: &str) -> Vec<u32> {
        let pattern = Pattern::new(query, CaseMatching::Smart, Normalization::Smart, AtomKind::Fuzzy);
        let mut out = Vec::new();
        pattern.indices(Utf32Str::new(candidate, &mut self.buf), &mut self.matcher, &mut out);
        out.sort_unstable();
        out.dedup();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_and_highlights() {
        let c =
            ["Projects/Garden plan.md", "Daily/2024-01-01.md", "gp.md", "Recipes/Green pasta.md", "zzz.md"];
        let mut f = Fuzzy::new();
        let hits = f.top("gp", &c, 3);
        assert_eq!(hits.len(), 3);
        assert_eq!(c[hits[0].index], "gp.md");
        assert!(hits.iter().all(|h| c[h.index] != "zzz.md"));
        assert_eq!(f.positions("gpl", "Garden plan.md"), vec![0, 7, 8]);
        assert!(f.top("qqq", &c, 3).is_empty());
        let first: Vec<usize> = f.top("", &c, 2).iter().map(|h| h.index).collect();
        assert_eq!(first, vec![0, 1]);
    }

    #[test]
    fn parallel_path_matches_sequential() {
        let names: Vec<String> = (0..20_000).map(|i| format!("dir{}/Note {i}.md", i % 37)).collect();
        let all: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut f = Fuzzy::new();
        let big = f.top("note 1234", &all, 10);
        // The same candidates in a list below the threshold must give the same ranking.
        let small: Vec<&str> = all[..8000].to_vec();
        let seq = f.top("note 1234", &small, 10);
        assert_eq!(all[big[0].index], "dir13/Note 1234.md");
        assert_eq!(small[seq[0].index], "dir13/Note 1234.md");
        assert_eq!(big.len(), 10);
    }
}
