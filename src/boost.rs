//! Boosting a vocabulary list (docs/design/20261022_04_STT_VOCAB_BOOST_PLAN.md):
//! a `TokenBooster` that nudges Kyutai STT toward the token sequences of
//! the phrases in a list, as it writes them.

use std::path::{Path, PathBuf};

use crate::spm::Decoder;

/// End of paragraph and end of stream: every match ends.
const ENDS: [u32; 2] = [0, 2];
/// Pad and silence pad: the model writes them between the pieces of one
/// word too, so they change nothing (xn-moshi `asr.rs`).
const PADS: [u32; 2] = [3, 4];

/// Default biases, added to the text logits (decision 4).
pub const START: f32 = 1.5;
pub const CONT: f32 = 8.0;

pub struct VocabBooster {
    /// Each phrase's tokens, as written and capitalized.
    phrases: Vec<Vec<u32>>,
    /// Matches in progress: (phrase, how many tokens matched).
    active: Vec<(usize, usize)>,
    /// Which tokens start a word (their piece begins with `▁`).
    word_starts: Vec<bool>,
    start: f32,
    cont: f32,
}

impl VocabBooster {
    /// `start` and `cont` from `VOICEXN_BOOST=start,cont`, else the defaults.
    pub fn biases() -> (f32, f32) {
        std::env::var("VOICEXN_BOOST")
            .ok()
            .and_then(|v| {
                let (a, b) = v.split_once(',')?;
                Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
            })
            .unwrap_or((START, CONT))
    }

    pub fn new(phrases: Vec<Vec<u32>>, word_starts: Vec<bool>, start: f32, cont: f32) -> Self {
        Self {
            phrases: phrases.into_iter().filter(|p| !p.is_empty()).collect(),
            active: Vec::new(),
            word_starts,
            start,
            cont,
        }
    }

    /// Encodes each phrase as written and with a capital first letter.
    pub fn from_phrases(decoder: &Decoder, phrases: &[String], start: f32, cont: f32) -> Self {
        let mut tokens = Vec::new();
        for phrase in phrases {
            let mut chars = phrase.chars();
            let capital: String = chars
                .next()
                .into_iter()
                .flat_map(char::to_uppercase)
                .chain(chars)
                .collect();
            for variant in [phrase.clone(), capital] {
                let ids = decoder.encode(&variant);
                if !tokens.contains(&ids) {
                    tokens.push(ids);
                }
            }
        }
        let word_starts = (0..decoder.len() as u32)
            .map(|id| decoder.starts_word(id))
            .collect();
        Self::new(tokens, word_starts, start, cont)
    }

    pub fn len(&self) -> usize {
        self.phrases.len()
    }

    pub fn is_empty(&self) -> bool {
        self.phrases.is_empty()
    }
}

impl xn_moshi::asr::TokenBooster for VocabBooster {
    fn on_text_token(&mut self, token: u32) {
        if ENDS.contains(&token) {
            self.active.clear();
            return;
        }
        if PADS.contains(&token) {
            return;
        }
        let phrases = &self.phrases;
        let mut active: Vec<(usize, usize)> = self
            .active
            .iter()
            .filter(|&&(p, at)| phrases[p][at] == token)
            .map(|&(p, at)| (p, at + 1))
            .collect();
        if self
            .word_starts
            .get(token as usize)
            .copied()
            .unwrap_or(false)
        {
            active.extend(
                phrases
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| p[0] == token)
                    .map(|(i, _)| (i, 1)),
            );
        }
        active.retain(|&(p, at)| at < phrases[p].len());
        self.active = active;
    }

    fn text_biases(&self) -> Vec<(u32, f32)> {
        let mut biases: Vec<(u32, f32)> = Vec::new();
        let mut add = |token: u32, bias: f32| match biases.iter_mut().find(|(t, _)| *t == token) {
            Some((_, b)) => *b = b.max(bias),
            None => biases.push((token, bias)),
        };
        for phrase in &self.phrases {
            add(phrase[0], self.start);
        }
        for &(p, at) in &self.active {
            add(self.phrases[p][at], self.cont);
        }
        biases
    }
}

/// The phrase list: `VOICEXN_VOCAB`, else `vocab.txt` in the working
/// directory (the desktop), else `vocab.txt` next to the model folder
/// (Android's data directory).
pub fn find_vocab(model_dir: &Path) -> Option<PathBuf> {
    if let Ok(path) = std::env::var("VOICEXN_VOCAB") {
        return Some(PathBuf::from(path));
    }
    [
        PathBuf::from("vocab.txt"),
        model_dir.parent()?.join("vocab.txt"),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

/// One phrase per line; blank lines and `#` comments are skipped.
pub fn read_vocab(path: &Path) -> std::io::Result<Vec<String>> {
    Ok(std::fs::read_to_string(path)?
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect())
}

#[cfg(test)]
mod tests {
    use xn_moshi::asr::TokenBooster;

    use super::*;

    fn booster() -> VocabBooster {
        // "▁lib mor ph" and "▁lib ms df", sharing their first token; tokens
        // 10 and 30 start words.
        let mut word_starts = vec![false; 100];
        word_starts[10] = true;
        word_starts[30] = true;
        VocabBooster::new(
            vec![vec![10, 11, 12], vec![10, 20, 21]],
            word_starts,
            1.5,
            4.0,
        )
    }

    fn sorted(mut b: Vec<(u32, f32)>) -> Vec<(u32, f32)> {
        b.sort_by_key(|&(t, _)| t);
        b
    }

    #[test]
    fn with_no_match_only_first_tokens() {
        let mut b = booster();
        assert_eq!(b.text_biases(), [(10, 1.5)]);
        b.on_text_token(30); // a word that is not on the list
        b.on_text_token(98);
        assert_eq!(b.text_biases(), [(10, 1.5)]);
    }

    #[test]
    fn a_match_boosts_its_next_token_across_pads() {
        let mut b = booster();
        b.on_text_token(10);
        assert_eq!(sorted(b.text_biases()), [(10, 1.5), (11, 4.0), (20, 4.0)]);
        b.on_text_token(3); // pad between pieces of the word
        b.on_text_token(4);
        b.on_text_token(11);
        assert_eq!(sorted(b.text_biases()), [(10, 1.5), (12, 4.0)]);
        b.on_text_token(12);
        assert_eq!(b.text_biases(), [(10, 1.5)], "the phrase is complete");
    }

    #[test]
    fn a_mismatch_or_an_end_drops_the_match() {
        let mut b = booster();
        b.on_text_token(10);
        b.on_text_token(77);
        assert_eq!(b.text_biases(), [(10, 1.5)]);
        b.on_text_token(10);
        b.on_text_token(0); // end of paragraph
        assert_eq!(b.text_biases(), [(10, 1.5)]);
        b.on_text_token(10);
        b.on_text_token(30); // a new word
        assert_eq!(b.text_biases(), [(10, 1.5)]);
    }

    #[test]
    fn a_continuation_piece_does_not_start_a_match() {
        let mut b = booster();
        b.on_text_token(30);
        b.on_text_token(10 + 90); // out of range: not a word start
        assert_eq!(b.text_biases(), [(10, 1.5)]);
    }

    #[test]
    fn the_real_tokenizer_gives_each_name_two_variants() {
        let path = Path::new("models/stt-1b-en_fr/tokenizer.model");
        if !path.exists() {
            return;
        }
        let d = Decoder::open(path).unwrap();
        let b = VocabBooster::from_phrases(&d, &["libmorph".into(), "libmsdf".into()], START, CONT);
        assert_eq!(b.len(), 4);
        assert!(b.phrases.iter().all(|p| p.len() >= 2), "{:?}", b.phrases);
    }
}
