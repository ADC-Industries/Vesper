//! The OPUS-MT engine: candle's Marian model on the CPU, one pair loaded at a time.
//!
//! Each line is encoded on its own (candle's Marian has no padding mask, and slide lines are
//! short) and decoded greedily (the padding token banned, at most three times the source
//! length). Work runs on a small pool of low-priority threads (a quarter of the cores, one to
//! four), never on the server's async runtime, so a translation never takes the cores a live
//! show renders and plays on. The model is dropped after [`IDLE`] without use.

use crate::catalogue::{Pair, find};
use crate::spm::{PieceTokenizer, Vocab};
use crate::store::{State, Store};
use crate::{ENGINE, Translator};
use anyhow::{Context, Result, anyhow, bail};
use candle_core::{DType, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::marian::{Config, MTModel};
use serde::Deserialize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A loaded model is dropped after this long without use.
pub const IDLE: Duration = Duration::from_secs(5 * 60);

/// The most pieces the engine reads at once. A longer line (a long Scripture paragraph) is
/// translated in parts of at most this many pieces, cut after a sentence where it can be
/// ([`chunks`]), and the parts joined again: nothing is left out.
const MAX_SOURCE: usize = 200;

/// A SentencePiece piece starting a word.
fn starts_word(piece: &str) -> bool {
    piece.starts_with('\u{2581}')
}

/// A piece ending a sentence (closing quotes and brackets allowed after the mark).
fn ends_sentence(piece: &str) -> bool {
    piece
        .trim_end_matches(['"', '\'', '\u{201d}', '\u{2019}', ')', '\u{bb}'])
        .ends_with(['.', '!', '?', ';', ':', '\u{3002}', '\u{ff01}', '\u{ff1f}'])
}

/// `pieces` in parts of at most `max`: each cut after the last sentence end in reach, else
/// before the last word in reach, else at `max`.
fn chunks(pieces: &[String], max: usize) -> Vec<&[String]> {
    let mut out = Vec::new();
    let mut rest = pieces;
    while rest.len() > max {
        let at = |good: &dyn Fn(usize) -> bool| (1..max).rev().find(|&i| good(i));
        let cut = at(&|i| ends_sentence(&rest[i - 1]) && starts_word(&rest[i]))
            .or_else(|| at(&|i| starts_word(&rest[i])))
            .unwrap_or(max);
        out.push(&rest[..cut]);
        rest = &rest[cut..];
    }
    if !rest.is_empty() {
        out.push(rest);
    }
    out
}

/// `config.json` of a Marian model, with the defaults older files leave out.
#[derive(Deserialize)]
struct ConfigFile {
    vocab_size: usize,
    #[serde(default)]
    decoder_vocab_size: Option<usize>,
    #[serde(default = "d512")]
    max_position_embeddings: usize,
    encoder_layers: usize,
    encoder_ffn_dim: usize,
    encoder_attention_heads: usize,
    decoder_layers: usize,
    decoder_ffn_dim: usize,
    decoder_attention_heads: usize,
    activation_function: candle_nn::Activation,
    d_model: usize,
    decoder_start_token_id: u32,
    #[serde(default = "yes")]
    scale_embedding: bool,
    pad_token_id: u32,
    eos_token_id: u32,
    #[serde(default)]
    forced_eos_token_id: Option<u32>,
}

fn d512() -> usize {
    512
}

fn yes() -> bool {
    true
}

impl ConfigFile {
    fn config(&self) -> Config {
        Config {
            vocab_size: self.vocab_size,
            decoder_vocab_size: self.decoder_vocab_size,
            max_position_embeddings: self.max_position_embeddings,
            encoder_layers: self.encoder_layers,
            encoder_ffn_dim: self.encoder_ffn_dim,
            encoder_attention_heads: self.encoder_attention_heads,
            decoder_layers: self.decoder_layers,
            decoder_ffn_dim: self.decoder_ffn_dim,
            decoder_attention_heads: self.decoder_attention_heads,
            use_cache: true,
            is_encoder_decoder: true,
            activation_function: self.activation_function,
            d_model: self.d_model,
            decoder_start_token_id: self.decoder_start_token_id,
            scale_embedding: self.scale_embedding,
            pad_token_id: self.pad_token_id,
            eos_token_id: self.eos_token_id,
            forced_eos_token_id: self.forced_eos_token_id.unwrap_or(self.eos_token_id),
            share_encoder_decoder_embeddings: true,
        }
    }
}

struct Loaded {
    pair: String,
    model: MTModel,
    cfg: Config,
    source: PieceTokenizer,
    vocab: Vocab,
    used: Instant,
}

/// The OPUS-MT engine over a [`Store`].
pub struct Marian {
    store: Arc<Store>,
    loaded: Mutex<Option<Loaded>>,
    pool: rayon::ThreadPool,
    threads: usize,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Lowers the calling thread's priority (translation yields to the show).
fn lower_priority() {
    #[cfg(unix)]
    // SAFETY: setpriority on the calling thread (Linux: `who` = its thread ID) only changes
    // its scheduling niceness.
    unsafe {
        #[cfg(target_os = "linux")]
        let who = libc::syscall(libc::SYS_gettid) as libc::id_t;
        #[cfg(not(target_os = "linux"))]
        let who = 0;
        libc::setpriority(libc::PRIO_PROCESS, who, 15);
    }
    #[cfg(windows)]
    // SAFETY: changes the calling thread's own priority.
    unsafe {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_LOWEST,
        };
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_LOWEST);
    }
}

impl Marian {
    /// The engine for the models in `store`.
    pub fn new(store: Arc<Store>) -> Result<Marian> {
        let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
        let threads = (cores / 4).clamp(1, 4);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|i| format!("translate-{i}"))
            .start_handler(|_| lower_priority())
            .build()?;
        Ok(Marian {
            store,
            loaded: Mutex::new(None),
            pool,
            threads,
        })
    }

    /// Threads it computes on.
    pub fn threads(&self) -> usize {
        self.threads
    }

    /// Drops the loaded model if unused for `idle`; true when one was dropped.
    pub fn unload_idle(&self, idle: Duration) -> bool {
        let mut l = lock(&self.loaded);
        if l.as_ref().is_some_and(|m| m.used.elapsed() >= idle) {
            *l = None;
            return true;
        }
        false
    }

    /// Drops pair `id`'s model if it is loaded (before its files are deleted).
    pub fn unload(&self, id: &str) {
        let mut l = lock(&self.loaded);
        if l.as_ref().is_some_and(|m| m.pair == id) {
            *l = None;
        }
    }

    /// The pair loaded now.
    pub fn loaded(&self) -> Option<String> {
        lock(&self.loaded).as_ref().map(|m| m.pair.clone())
    }

    fn load(&self, pair: &Pair) -> Result<Loaded> {
        let dir = self.store.path(&pair.id);
        let cfg: ConfigFile = serde_json::from_slice(&std::fs::read(dir.join("config.json"))?)
            .context("reading the model's config.json")?;
        let cfg = cfg.config();
        let device = Device::Cpu;
        let mut weights = if pair.files.iter().any(|f| f.name == "model.safetensors") {
            candle_core::safetensors::load(dir.join("model.safetensors"), &device)?
        } else {
            let bin = dir.join("pytorch_model.bin");
            if crate::pth::is_legacy(&bin) {
                crate::pth::load(&bin, &device)?
            } else {
                candle_core::pickle::read_all(&bin)?.into_iter().collect()
            }
        };
        // Tied embeddings: a file may store the shared table under one of its aliases only.
        if !weights.contains_key("model.shared.weight") {
            let alias = [
                "model.decoder.embed_tokens.weight",
                "model.encoder.embed_tokens.weight",
            ]
            .into_iter()
            .find_map(|k| weights.get(k).cloned());
            if let Some(t) = alias {
                weights.insert("model.shared.weight".into(), t);
            }
        }
        let vb = VarBuilder::from_tensors(weights, DType::F32, &device);
        let model = MTModel::new(&cfg, vb).context("loading the model's weights")?;
        Ok(Loaded {
            pair: pair.id.clone(),
            model,
            cfg,
            source: PieceTokenizer::load(&dir.join("source.spm"))?,
            vocab: Vocab::load(&dir.join("vocab.json"))?,
            used: Instant::now(),
        })
    }

    /// One source line translated (in parts when it is long: [`chunks`]).
    fn line(m: &mut Loaded, token: Option<&str>, text: &str) -> Result<String> {
        let pieces = m.source.pieces(text)?;
        let mut parts = Vec::new();
        for c in chunks(&pieces, MAX_SOURCE) {
            parts.push(Marian::part(m, token, c)?);
        }
        Ok(parts.join(" "))
    }

    /// At most [`MAX_SOURCE`] pieces translated.
    fn part(m: &mut Loaded, token: Option<&str>, pieces: &[String]) -> Result<String> {
        let unk = m.vocab.id("<unk>").unwrap_or(1);
        let mut ids: Vec<u32> = Vec::new();
        if let Some(t) = token {
            ids.push(
                m.vocab
                    .id(t)
                    .ok_or_else(|| anyhow!("the model has no target token {t}"))?,
            );
        }
        for p in pieces {
            ids.push(m.vocab.id(p).unwrap_or(unk));
        }
        ids.push(m.cfg.eos_token_id);
        let device = Device::Cpu;
        let input = Tensor::new(ids.as_slice(), &device)?.unsqueeze(0)?;
        m.model.reset_kv_cache();
        let encoded = m.model.encoder().forward(&input, 0)?;
        let max = (ids.len() * 3 + 10).min(m.cfg.max_position_embeddings - 1);
        let mut out = vec![m.cfg.decoder_start_token_id];
        let pad = m.cfg.pad_token_id as usize;
        for i in 0..max {
            let context = if i >= 1 { 1 } else { out.len() };
            let start = out.len().saturating_sub(context);
            let input = Tensor::new(&out[start..], &device)?.unsqueeze(0)?;
            let logits = m.model.decode(&input, &encoded, start)?;
            let logits = logits.squeeze(0)?;
            let logits = logits.i(logits.dim(0)? - 1)?;
            let mut v: Vec<f32> = logits.to_vec1()?;
            if let Some(x) = v.get_mut(pad) {
                *x = f32::NEG_INFINITY;
            }
            let next = v
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map(|(i, _)| i as u32)
                .ok_or_else(|| anyhow!("empty logits"))?;
            out.push(next);
            if next == m.cfg.eos_token_id || next == m.cfg.forced_eos_token_id {
                break;
            }
        }
        m.model.reset_kv_cache();
        Ok(m.vocab.decode(&out[1..]))
    }
}

impl Translator for Marian {
    fn translate(&self, from: &str, to: &str, lines: &[String]) -> Result<Vec<String>> {
        let pair = find(self.store.pairs(), from, to)
            .ok_or_else(|| anyhow!("no model translates {from} to {to}"))?
            .clone();
        if self.store.state(&pair.id) != State::Ready {
            bail!("the {} model is not downloaded", pair.id);
        }
        let token = pair.target(to).and_then(|t| t.token.clone());
        self.pool.install(|| {
            let mut slot = lock(&self.loaded);
            if slot.as_ref().is_none_or(|m| m.pair != pair.id) {
                *slot = None;
                let t = Instant::now();
                *slot = Some(self.load(&pair)?);
                tracing::info!(pair = %pair.id, "translation model loaded in {:?}", t.elapsed());
            }
            let m = slot.as_mut().expect("loaded");
            m.used = Instant::now();
            let r = lines
                .iter()
                .map(|l| Marian::line(m, token.as_deref(), l))
                .collect::<Result<Vec<_>>>();
            m.used = Instant::now();
            r
        })
    }

    fn engine(&self) -> String {
        ENGINE.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pieces(text: &str) -> Vec<String> {
        text.split(' ').map(|w| format!("\u{2581}{w}")).collect()
    }

    fn words(c: &[String]) -> String {
        c.iter()
            .map(|p| p.trim_start_matches('\u{2581}'))
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn a_long_line_is_translated_in_parts_without_losing_any() {
        // Short lines are one part.
        let p = pieces("In the beginning was the Word");
        assert_eq!(chunks(&p, 200), vec![&p[..]]);
        // Long ones are cut after a sentence within reach.
        let p = pieces("one two. three four five! six seven eight nine ten");
        let c = chunks(&p, 6);
        assert_eq!(
            c.iter().map(|c| words(c)).collect::<Vec<_>>(),
            ["one two. three four five!", "six seven eight nine ten"]
        );
        assert!(c.iter().all(|c| c.len() <= 6));
        assert_eq!(c.concat(), p);
        // Without a sentence end: before a word; one long word: at the limit.
        let mut p = pieces("alpha beta");
        p.insert(1, "lph".into());
        p.insert(1, "lph".into());
        let c = chunks(&p, 3);
        assert_eq!(c.concat(), p);
        assert_eq!(c[0].len(), 3);
        assert!(c.iter().all(|c| c.len() <= 3));
        // A 700-word paragraph: every piece is kept, in parts of at most MAX_SOURCE.
        let long: Vec<String> = (0..700)
            .map(|i| format!("\u{2581}w{i}{}", if i % 23 == 22 { "." } else { "" }))
            .collect();
        let c = chunks(&long, MAX_SOURCE);
        assert!(c.len() >= 4);
        assert!(c.iter().all(|c| c.len() <= MAX_SOURCE));
        assert!(
            c[..c.len() - 1]
                .iter()
                .all(|c| c.last().unwrap().ends_with('.'))
        );
        assert_eq!(c.concat(), long);
    }
}
