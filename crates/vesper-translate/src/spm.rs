//! Marian's tokenizer, built from the files the Helsinki-NLP repositories ship.
//!
//! The repositories have no `tokenizer.json`, only SentencePiece models (`source.spm`,
//! `target.spm`) and the model's vocabulary (`vocab.json`). Instead of a converted tokenizer
//! (candle's Marian example downloads one made with a Python conversion script), this reads the
//! SentencePiece model itself: its pieces and scores and its normalisation table (a small
//! protobuf reader of the public `sentencepiece_model.proto` fields), and builds the same
//! Unigram tokenizer with Hugging Face's `tokenizers` crate (Apache-2.0, already a dependency of
//! candle): the precompiled normaliser, runs of spaces collapsed, `▁` word marks, Unigram
//! Viterbi. Pieces are then numbered by `vocab.json`, as Marian's tokenizer does. Decoding
//! needs only the vocabulary: pieces joined, `▁` back to spaces.

use anyhow::{Context, Result, anyhow, bail};
use std::collections::HashMap;
use std::path::Path;
use tokenizers::models::unigram::Unigram;
use tokenizers::normalizers::{NormalizerWrapper, Precompiled, Replace, Sequence};
use tokenizers::pre_tokenizers::metaspace::{Metaspace, PrependScheme};

/// The word mark SentencePiece puts for a space.
pub const MARK: char = '\u{2581}';

/// One piece of a SentencePiece model.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    /// Its text.
    pub piece: String,
    /// Its log probability.
    pub score: f32,
    /// SentencePiece type: 1 normal, 2 unknown, 3 control, 4 user-defined, 6 byte.
    pub kind: u32,
}

/// What the tokenizer needs from a SentencePiece model.
#[derive(Clone, Debug, Default)]
pub struct SpmModel {
    /// The pieces, in ID order.
    pub pieces: Vec<Piece>,
    /// The normaliser's precompiled character map (may be empty: no normalisation).
    pub charsmap: Vec<u8>,
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn varint(&mut self) -> Result<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = *self
                .b
                .get(self.i)
                .ok_or_else(|| anyhow!("truncated varint"))?;
            self.i += 1;
            v |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(v);
            }
        }
        bail!("varint too long")
    }

    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.i.checked_add(n).filter(|e| *e <= self.b.len());
        let end = end.ok_or_else(|| anyhow!("truncated field"))?;
        let s = &self.b[self.i..end];
        self.i = end;
        Ok(s)
    }

    /// The next field: (number, wire type, its bytes for length-delimited and fixed fields,
    /// its value for varints).
    fn field(&mut self) -> Result<Option<(u64, Field<'a>)>> {
        if self.i >= self.b.len() {
            return Ok(None);
        }
        let key = self.varint()?;
        let (num, wire) = (key >> 3, key & 7);
        let f = match wire {
            0 => Field::Varint(self.varint()?),
            1 => Field::Bytes(self.bytes(8)?),
            2 => {
                let n = usize::try_from(self.varint()?)?;
                Field::Bytes(self.bytes(n)?)
            }
            5 => Field::Bytes(self.bytes(4)?),
            w => bail!("unsupported protobuf wire type {w}"),
        };
        Ok(Some((num, f)))
    }
}

enum Field<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
}

/// Reads a SentencePiece model (`ModelProto`: field 1 the pieces, field 3 the normaliser).
pub fn parse(bytes: &[u8]) -> Result<SpmModel> {
    let mut m = SpmModel::default();
    let mut r = Reader { b: bytes, i: 0 };
    while let Some((num, f)) = r.field()? {
        match (num, f) {
            (1, Field::Bytes(b)) => m.pieces.push(piece(b)?),
            (3, Field::Bytes(b)) => {
                let mut n = Reader { b, i: 0 };
                while let Some((num, f)) = n.field()? {
                    if let (2, Field::Bytes(map)) = (num, f) {
                        m.charsmap = map.to_vec();
                    }
                }
            }
            _ => {}
        }
    }
    if m.pieces.is_empty() {
        bail!("not a SentencePiece model (no pieces)");
    }
    Ok(m)
}

fn piece(b: &[u8]) -> Result<Piece> {
    let mut p = Piece {
        piece: String::new(),
        score: 0.0,
        kind: 1,
    };
    let mut r = Reader { b, i: 0 };
    while let Some((num, f)) = r.field()? {
        match (num, f) {
            (1, Field::Bytes(s)) => p.piece = String::from_utf8(s.to_vec())?,
            (2, Field::Bytes(s)) if s.len() == 4 => {
                p.score = f32::from_le_bytes([s[0], s[1], s[2], s[3]]);
            }
            (3, Field::Varint(v)) => p.kind = v as u32,
            _ => {}
        }
    }
    Ok(p)
}

/// Splits text into pieces as the SentencePiece model would.
pub struct PieceTokenizer {
    inner: tokenizers::Tokenizer,
}

impl PieceTokenizer {
    /// A tokenizer for the SentencePiece model in `spm`.
    pub fn new(spm: &SpmModel) -> Result<PieceTokenizer> {
        let unk = spm
            .pieces
            .iter()
            .position(|p| p.kind == 2)
            .ok_or_else(|| anyhow!("the SentencePiece model has no unknown piece"))?;
        let vocab = spm
            .pieces
            .iter()
            .map(|p| (p.piece.clone(), f64::from(p.score)))
            .collect();
        let model = Unigram::from(vocab, Some(unk), false).map_err(|e| anyhow!("{e}"))?;
        let mut t = tokenizers::Tokenizer::new(model);
        let mut steps: Vec<NormalizerWrapper> = Vec::new();
        if !spm.charsmap.is_empty() {
            let p = Precompiled::from(&spm.charsmap).map_err(|e| anyhow!("{e}"))?;
            steps.push(p.into());
        }
        let spaces = Replace::new(
            tokenizers::normalizers::replace::ReplacePattern::Regex(" {2,}".into()),
            " ",
        )
        .map_err(|e| anyhow!("{e}"))?;
        steps.push(spaces.into());
        t.with_normalizer(Some(Sequence::new(steps)))
            .map_err(|e| anyhow!("{e}"))?;
        t.with_pre_tokenizer(Some(Metaspace::new(MARK, PrependScheme::Always, true)));
        Ok(PieceTokenizer { inner: t })
    }

    /// Reads `source.spm`.
    pub fn load(path: &Path) -> Result<PieceTokenizer> {
        let b = std::fs::read(path).with_context(|| format!("reading {path:?}"))?;
        PieceTokenizer::new(&parse(&b)?)
    }

    /// The pieces of `text`.
    pub fn pieces(&self, text: &str) -> Result<Vec<String>> {
        let e = self
            .inner
            .encode(text, false)
            .map_err(|e| anyhow!("tokenizing: {e}"))?;
        Ok(e.get_tokens().to_vec())
    }
}

/// Marian's vocabulary (`vocab.json`): piece ↔ ID.
pub struct Vocab {
    ids: HashMap<String, u32>,
    pieces: Vec<String>,
}

impl Vocab {
    /// Reads `vocab.json`.
    pub fn load(path: &Path) -> Result<Vocab> {
        let b = std::fs::read(path).with_context(|| format!("reading {path:?}"))?;
        Vocab::from_json(&b)
    }

    /// From `vocab.json`'s bytes.
    pub fn from_json(b: &[u8]) -> Result<Vocab> {
        let ids: HashMap<String, u32> = serde_json::from_slice(b)?;
        let n = ids.values().max().map_or(0, |m| *m as usize + 1);
        let mut pieces = vec![String::new(); n];
        for (p, i) in &ids {
            pieces[*i as usize] = p.clone();
        }
        Ok(Vocab { ids, pieces })
    }

    /// The ID of `piece`.
    pub fn id(&self, piece: &str) -> Option<u32> {
        self.ids.get(piece).copied()
    }

    /// The piece of `id`.
    pub fn piece(&self, id: u32) -> Option<&str> {
        self.pieces.get(id as usize).map(String::as_str)
    }

    /// The text of target IDs (special pieces such as `</s>`, `<pad>`, `<unk>` and `>>xx<<`
    /// left out).
    pub fn decode(&self, ids: &[u32]) -> String {
        let mut s = String::new();
        for id in ids {
            let Some(p) = self.piece(*id) else { continue };
            if (p.starts_with('<') && p.ends_with('>'))
                || (p.starts_with(">>") && p.ends_with("<<"))
            {
                continue;
            }
            s.push_str(p);
        }
        s.replace(MARK, " ").trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn varint(mut v: u64, out: &mut Vec<u8>) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                return;
            }
            out.push(b | 0x80);
        }
    }

    fn field(num: u64, wire: u64, out: &mut Vec<u8>) {
        varint(num << 3 | wire, out);
    }

    fn piece_bytes(p: &str, score: f32, kind: u64) -> Vec<u8> {
        let mut b = Vec::new();
        field(1, 2, &mut b);
        varint(p.len() as u64, &mut b);
        b.extend_from_slice(p.as_bytes());
        field(2, 5, &mut b);
        b.extend_from_slice(&score.to_le_bytes());
        if kind != 1 {
            field(3, 0, &mut b);
            varint(kind, &mut b);
        }
        b
    }

    /// A made-up SentencePiece model: whole words and letters.
    fn model() -> Vec<u8> {
        let mut out = Vec::new();
        let mut pieces = vec![("<unk>", 0.0, 2), ("<s>", 0.0, 3), ("</s>", 0.0, 3)];
        pieces.extend([
            ("\u{2581}grace", -2.0, 1),
            ("\u{2581}and", -2.0, 1),
            ("\u{2581}peace", -2.0, 1),
            ("\u{2581}", -3.0, 1),
        ]);
        let letters: Vec<String> = "abcdefghijklmnopqrstuvwxyz"
            .chars()
            .map(String::from)
            .collect();
        for l in &letters {
            pieces.push((l.as_str(), -5.0, 1));
        }
        for (p, s, k) in pieces {
            let b = piece_bytes(p, s, k);
            field(1, 2, &mut out);
            varint(b.len() as u64, &mut out);
            out.extend(b);
        }
        // A trainer spec (field 2) the reader skips.
        field(2, 2, &mut out);
        varint(2, &mut out);
        out.extend([8, 1]);
        out
    }

    #[test]
    fn reads_pieces_and_splits_words() {
        let m = parse(&model()).unwrap();
        assert_eq!(m.pieces.len(), 7 + 26);
        assert_eq!(m.pieces[0].kind, 2);
        assert_eq!(m.pieces[3].piece, "\u{2581}grace");
        assert_eq!(m.pieces[3].score, -2.0);
        let t = PieceTokenizer::new(&m).unwrap();
        assert_eq!(
            t.pieces("grace  and peace").unwrap(),
            vec!["\u{2581}grace", "\u{2581}and", "\u{2581}peace"]
        );
        let p = t.pieces("graces").unwrap();
        assert_eq!(p, vec!["\u{2581}grace", "s"]);
        assert!(parse(&[]).is_err());
        assert!(parse(&[0x0a, 0x05, 1]).is_err());
    }

    #[test]
    fn the_vocabulary_numbers_and_joins_pieces() {
        let v = Vocab::from_json(
            r#"{"</s>":0,"<unk>":1,"▁gracia":2,"▁y":3,"▁paz":4,">>spa<<":5,"<pad>":6}"#.as_bytes(),
        )
        .unwrap();
        assert_eq!(v.id("\u{2581}paz"), Some(4));
        assert_eq!(v.decode(&[5, 2, 3, 4, 0, 6]), "gracia y paz");
    }
}
