//! SentencePiece in Rust: decoding ids to text (plan 20261022_02,
//! decision 7), and unigram encoding of text to ids (plan 20261022_04,
//! decision 3). A `.model` file is a protobuf `ModelProto` whose field 1
//! repeats `SentencePiece { piece: string = 1, score: float = 2, type:
//! enum = 3 }`; this reads only those pieces.

use std::collections::HashMap;
use std::path::Path;

/// `SentencePiece.Type` values that decode to something other than the
/// piece's own text.
const UNKNOWN: u64 = 2;
const CONTROL: u64 = 3;
const BYTE: u64 = 6;

/// The word boundary marker, U+2581.
const SPACE: char = '\u{2581}';

#[derive(Clone, Debug, PartialEq)]
enum Piece {
    Text(String),
    Byte(u8),
    /// Control and unknown pieces decode to nothing.
    Empty,
}

pub struct Decoder {
    pieces: Vec<Piece>,
    /// Text pieces by their text, with id and score, for encoding.
    vocab: HashMap<String, (u32, f32)>,
    /// The `<0xNN>` piece of each byte, for characters no piece covers.
    bytes: [Option<u32>; 256],
    /// The longest text piece, in characters.
    max_chars: usize,
}

/// The score of a character spelt in byte pieces, below any real piece.
const BYTE_FALLBACK_SCORE: f32 = -100.0;

#[derive(Debug)]
pub struct Error(String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

fn err(what: &str) -> Error {
    Error(format!("sentencepiece model: {what}"))
}

/// A protobuf reader over one message's bytes.
struct Fields<'a>(&'a [u8]);

impl<'a> Fields<'a> {
    fn varint(&mut self) -> Result<u64, Error> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let (&byte, rest) = self
                .0
                .split_first()
                .ok_or_else(|| err("truncated varint"))?;
            self.0 = rest;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(err("varint too long"))
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        if n > self.0.len() {
            return Err(err("truncated field"));
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }

    /// The next field as `(number, wire type, varint value or bytes)`.
    fn next(&mut self) -> Option<Result<(u64, Value<'a>), Error>> {
        if self.0.is_empty() {
            return None;
        }
        Some((|| {
            let key = self.varint()?;
            let value = match key & 7 {
                0 => Value::Varint(self.varint()?),
                1 => Value::Bytes(self.take(8)?),
                2 => {
                    let n = self.varint()? as usize;
                    Value::Bytes(self.take(n)?)
                }
                5 => Value::Bytes(self.take(4)?),
                wire => return Err(err(&format!("wire type {wire}"))),
            };
            Ok((key >> 3, value))
        })())
    }
}

enum Value<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
}

fn piece(bytes: &[u8]) -> Result<(Piece, f32), Error> {
    let mut text = None;
    let mut kind = 1;
    let mut score = 0.0;
    let mut fields = Fields(bytes);
    while let Some(field) = fields.next() {
        match field? {
            (1, Value::Bytes(b)) => {
                text = Some(String::from_utf8(b.to_vec()).map_err(|_| err("piece is not UTF-8"))?);
            }
            (2, Value::Bytes(b)) if b.len() == 4 => {
                score = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            }
            (3, Value::Varint(v)) => kind = v,
            _ => {}
        }
    }
    let text = text.ok_or_else(|| err("piece without text"))?;
    let piece = match kind {
        UNKNOWN | CONTROL => Piece::Empty,
        BYTE => text
            .strip_prefix("<0x")
            .and_then(|t| t.strip_suffix('>'))
            .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            .map_or(Piece::Empty, Piece::Byte),
        _ => Piece::Text(text),
    };
    Ok((piece, score))
}

impl Decoder {
    pub fn from_bytes(model: &[u8]) -> Result<Self, Error> {
        let mut pieces = Vec::new();
        let mut vocab = HashMap::new();
        let mut bytes = [None; 256];
        let mut max_chars = 1;
        let mut fields = Fields(model);
        while let Some(field) = fields.next() {
            if let (1, Value::Bytes(b)) = field? {
                let (piece, score) = piece(b)?;
                let id = pieces.len() as u32;
                match &piece {
                    Piece::Text(text) => {
                        max_chars = max_chars.max(text.chars().count());
                        vocab.insert(text.clone(), (id, score));
                    }
                    Piece::Byte(b) => bytes[usize::from(*b)] = Some(id),
                    Piece::Empty => {}
                }
                pieces.push(piece);
            }
        }
        if pieces.is_empty() {
            return Err(err("no pieces"));
        }
        Ok(Self {
            pieces,
            vocab,
            bytes,
            max_chars: max_chars.min(16),
        })
    }

    pub fn open(path: &Path) -> Result<Self, Error> {
        let bytes = std::fs::read(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
        Self::from_bytes(&bytes)
    }

    pub fn len(&self) -> usize {
        self.pieces.len()
    }

    /// Whether piece `id` starts a word: its text begins with `▁`.
    pub fn starts_word(&self, id: u32) -> bool {
        matches!(self.pieces.get(id as usize), Some(Piece::Text(t)) if t.starts_with(SPACE))
    }

    pub fn is_empty(&self) -> bool {
        self.pieces.is_empty()
    }

    /// The text of `ids`, with word markers as spaces and no leading space.
    /// Ids out of range decode to nothing.
    pub fn decode(&self, ids: &[u32]) -> String {
        let mut bytes = Vec::new();
        for &id in ids {
            match self.pieces.get(id as usize) {
                Some(Piece::Text(t)) => bytes.extend_from_slice(t.as_bytes()),
                Some(Piece::Byte(b)) => bytes.push(*b),
                Some(Piece::Empty) | None => {}
            }
        }
        let text = String::from_utf8_lossy(&bytes).replace(SPACE, " ");
        text.trim_start_matches(' ').to_string()
    }

    /// The ids of `text` as the unigram model segments it: the split into
    /// pieces with the largest total score, of `▁` + `text` with spaces as
    /// `▁`. A character no piece covers is spelt in byte pieces, and
    /// dropped if the model has none for it.
    pub fn encode(&self, text: &str) -> Vec<u32> {
        let text: String = std::iter::once(SPACE)
            .chain(text.chars().map(|c| if c == ' ' { SPACE } else { c }))
            .collect();
        let starts: Vec<usize> = text
            .char_indices()
            .map(|(i, _)| i)
            .chain([text.len()])
            .collect();
        let n = starts.len() - 1;
        // best[i]: the best score of the first i characters, and the step
        // that reached it (where it came from and the ids it added).
        let mut best: Vec<Option<(f32, usize, Vec<u32>)>> = vec![None; n + 1];
        best[0] = Some((0.0, 0, Vec::new()));
        for i in 0..n {
            let Some((at, _, _)) = best[i] else {
                continue;
            };
            let mut reach = |j: usize, score: f32, ids: Vec<u32>| {
                if best[j].as_ref().is_none_or(|(s, _, _)| score > *s) {
                    best[j] = Some((score, i, ids));
                }
            };
            let mut covered = false;
            for len in 1..=self.max_chars.min(n - i) {
                let piece = &text[starts[i]..starts[i + len]];
                if let Some(&(id, score)) = self.vocab.get(piece) {
                    covered |= len == 1;
                    reach(i + len, at + score, vec![id]);
                }
            }
            if !covered {
                let ids: Vec<u32> = text[starts[i]..starts[i + 1]]
                    .bytes()
                    .filter_map(|b| self.bytes[usize::from(b)])
                    .collect();
                reach(i + 1, at + BYTE_FALLBACK_SCORE, ids);
            }
        }
        let mut ids = Vec::new();
        let mut j = n;
        while j > 0 {
            let (_, i, step) = best[j].take().expect("every position is reachable");
            ids.splice(0..0, step);
            j = i;
        }
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encodes a `ModelProto` with these pieces, as the protobuf would.
    fn model(pieces: &[(&str, u64)]) -> Vec<u8> {
        let scored: Vec<_> = pieces.iter().map(|&(t, k)| (t, k, 0.5)).collect();
        model_scored(&scored)
    }

    fn model_scored(pieces: &[(&str, u64, f32)]) -> Vec<u8> {
        fn varint(out: &mut Vec<u8>, mut v: u64) {
            while v >= 0x80 {
                out.push((v as u8) | 0x80);
                v >>= 7;
            }
            out.push(v as u8);
        }
        let mut out = Vec::new();
        for &(text, kind, score) in pieces {
            let mut p = Vec::new();
            p.push(1 << 3 | 2);
            varint(&mut p, text.len() as u64);
            p.extend_from_slice(text.as_bytes());
            p.push(2 << 3 | 5); // score, ignored
            p.extend_from_slice(&score.to_le_bytes());
            p.push(3 << 3);
            varint(&mut p, kind);
            out.push(1 << 3 | 2);
            varint(&mut out, p.len() as u64);
            out.extend_from_slice(&p);
        }
        // A trainer_spec (field 2) that the decoder skips.
        out.extend_from_slice(&[2 << 3 | 2, 2, 8, 1]);
        out
    }

    #[test]
    fn pieces_bytes_and_controls_decode() {
        let d = Decoder::from_bytes(&model(&[
            ("<unk>", UNKNOWN),
            ("<s>", CONTROL),
            ("\u{2581}hello", 1),
            ("\u{2581}wor", 1),
            ("ld", 1),
            ("<0xC3>", BYTE),
            ("<0xA9>", BYTE),
        ]))
        .unwrap();
        assert_eq!(d.len(), 7);
        assert_eq!(d.decode(&[1, 2, 3, 4, 0]), "hello world");
        assert_eq!(d.decode(&[3, 5, 6]), "wor\u{e9}");
        assert_eq!(d.decode(&[99]), "");
    }

    #[test]
    fn a_truncated_model_is_an_error() {
        let bytes = model(&[("\u{2581}a", 1)]);
        assert!(Decoder::from_bytes(&bytes[..bytes.len() - 6]).is_err());
    }

    #[test]
    fn the_best_segmentation_beats_the_greedy_one() {
        let d = Decoder::from_bytes(&model_scored(&[
            ("<unk>", UNKNOWN, 0.0),
            ("\u{2581}", 1, -5.0),
            ("\u{2581}ab", 1, -1.0),
            ("c", 1, -9.0),
            ("\u{2581}a", 1, -2.0),
            ("bc", 1, -1.0),
            ("a", 1, -5.0),
            ("b", 1, -5.0),
        ]))
        .unwrap();
        // Greedy longest-first would take ▁ab + c (-10); ▁a + bc is -3.
        assert_eq!(d.encode("abc"), [4, 5]);
        assert_eq!(d.decode(&d.encode("abc")), "abc");
        assert_eq!(d.encode("ab"), [2]);
    }

    #[test]
    fn characters_without_a_piece_fall_back_to_bytes() {
        let d = Decoder::from_bytes(&model(&[
            ("\u{2581}", 1),
            ("<0xC3>", BYTE),
            ("<0xA9>", BYTE),
        ]))
        .unwrap();
        assert_eq!(d.encode("\u{e9}"), [0, 1, 2]);
        assert_eq!(d.decode(&d.encode("\u{e9}")), "\u{e9}");
    }

    /// Against the real tokenizer, when `scripts/fetch-stt.sh` has run.
    #[test]
    fn the_stt_tokenizer_round_trips() {
        let path = Path::new("models/stt-1b-en_fr/tokenizer.model");
        if !path.exists() {
            return;
        }
        let d = Decoder::open(path).unwrap();
        assert_eq!(d.len(), 8000);
        for s in [
            "libmorph",
            "Libmorph",
            "libcontrols libmsdf libtypedmd",
            "Phonon turns text into natural speech, right on your device.",
        ] {
            let ids = d.encode(s);
            assert_eq!(d.decode(&ids), s, "{ids:?}");
        }
    }
}
