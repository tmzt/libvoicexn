//! Decoding SentencePiece ids to text, in Rust (plan 20261022_02,
//! decision 7). A `.model` file is a protobuf `ModelProto` whose field 1
//! repeats `SentencePiece { piece: string = 1, score: float = 2, type:
//! enum = 3 }`; this reads only those pieces. It does not encode.

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
}

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

fn piece(bytes: &[u8]) -> Result<Piece, Error> {
    let mut text = None;
    let mut kind = 1;
    let mut fields = Fields(bytes);
    while let Some(field) = fields.next() {
        match field? {
            (1, Value::Bytes(b)) => {
                text = Some(String::from_utf8(b.to_vec()).map_err(|_| err("piece is not UTF-8"))?);
            }
            (3, Value::Varint(v)) => kind = v,
            _ => {}
        }
    }
    let text = text.ok_or_else(|| err("piece without text"))?;
    Ok(match kind {
        UNKNOWN | CONTROL => Piece::Empty,
        BYTE => text
            .strip_prefix("<0x")
            .and_then(|t| t.strip_suffix('>'))
            .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            .map_or(Piece::Empty, Piece::Byte),
        _ => Piece::Text(text),
    })
}

impl Decoder {
    pub fn from_bytes(model: &[u8]) -> Result<Self, Error> {
        let mut pieces = Vec::new();
        let mut fields = Fields(model);
        while let Some(field) = fields.next() {
            if let (1, Value::Bytes(b)) = field? {
                pieces.push(piece(b)?);
            }
        }
        if pieces.is_empty() {
            return Err(err("no pieces"));
        }
        Ok(Self { pieces })
    }

    pub fn open(path: &Path) -> Result<Self, Error> {
        let bytes = std::fs::read(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
        Self::from_bytes(&bytes)
    }

    pub fn len(&self) -> usize {
        self.pieces.len()
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encodes a `ModelProto` with these pieces, as the protobuf would.
    fn model(pieces: &[(&str, u64)]) -> Vec<u8> {
        fn varint(out: &mut Vec<u8>, mut v: u64) {
            while v >= 0x80 {
                out.push((v as u8) | 0x80);
                v >>= 7;
            }
            out.push(v as u8);
        }
        let mut out = Vec::new();
        for &(text, kind) in pieces {
            let mut p = Vec::new();
            p.push(1 << 3 | 2);
            varint(&mut p, text.len() as u64);
            p.extend_from_slice(text.as_bytes());
            p.push(2 << 3 | 5); // score, ignored
            p.extend_from_slice(&0.5f32.to_le_bytes());
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

    /// Against the real tokenizer, when `scripts/fetch-stt.sh` has run.
    #[test]
    fn the_stt_tokenizer_loads() {
        let path = Path::new("models/stt-1b-en_fr/tokenizer.model");
        if !path.exists() {
            return;
        }
        let d = Decoder::open(path).unwrap();
        assert_eq!(d.len(), 8000);
    }
}
