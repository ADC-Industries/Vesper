//! Weights in PyTorch's legacy (pre-1.6, not zip) `torch.save` format, which most of the
//! Helsinki-NLP repositories' `pytorch_model.bin` files use; candle reads only the zip format.
//!
//! The legacy file is five pickles and then the raw storages: the magic number, the protocol
//! version, a system info dict, the state dict (each tensor a `_rebuild_tensor_v2` call whose
//! storage is a persistent ID `('storage', type, key, location, numel)`), the list of storage
//! keys in file order; then, for each key, its element count (u64, little-endian) and its
//! bytes. candle's own pickle reader (which interprets a fixed set of opcodes and runs no
//! Python) reads the pickles; tensors are cut from their storages. Only contiguous tensors (all
//! a state dict holds) are taken.

use anyhow::{Context, Result, anyhow, bail};
use candle_core::pickle::{Object, Stack};
use candle_core::{DType, Device, Tensor};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

fn pickle<R: BufRead>(r: &mut R) -> Result<Object> {
    let mut s = Stack::empty();
    s.read_loop(r)?;
    Ok(s.finalize()?)
}

/// True when `path` starts like a legacy `torch.save` file (not a zip archive).
pub fn is_legacy(path: &Path) -> bool {
    let mut b = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut b))
        .is_ok()
        && b[0] == 0x80
        && b[2] == 0x8a
}

/// Every tensor of a legacy file, by name.
pub fn load(path: &Path, device: &Device) -> Result<HashMap<String, Tensor>> {
    let f = std::fs::File::open(path).with_context(|| format!("opening {path:?}"))?;
    let mut r = BufReader::with_capacity(1 << 20, f);
    let magic = pickle(&mut r)?;
    match magic {
        Object::Int(_) | Object::Long(_) => {}
        _ => bail!("not a legacy PyTorch file"),
    }
    let _protocol = pickle(&mut r)?;
    let _sys_info = pickle(&mut r)?;
    let state = pickle(&mut r)?;
    let keys: Vec<String> = match pickle(&mut r)? {
        Object::List(v) | Object::Tuple(v) => v
            .into_iter()
            .map(|k| match k {
                Object::Unicode(s) => Ok(s),
                other => Err(anyhow!("a storage key is not a string: {other:?}")),
            })
            .collect::<Result<_>>()?,
        other => bail!("the storage key list is not a list: {other:?}"),
    };
    let Object::Dict(entries) = state else {
        bail!("the weights are not a state dict");
    };
    let dir = Path::new("");
    let mut infos = Vec::new();
    for (name, value) in entries {
        if let Some(info) = value.into_tensor_info(name, dir)? {
            infos.push(info);
        }
    }
    // The storages, in file order: element count, then the bytes.
    let dtype_of: HashMap<String, DType> = infos
        .iter()
        .map(|i| (i.path.trim_start_matches('/').to_string(), i.dtype))
        .collect();
    let mut storages: HashMap<String, Vec<u8>> = HashMap::new();
    for k in keys {
        let mut n = [0u8; 8];
        r.read_exact(&mut n)?;
        let numel = u64::from_le_bytes(n) as usize;
        let size = dtype_of
            .get(&k)
            .map_or(4, |d| d.size_in_bytes())
            .checked_mul(numel)
            .ok_or_else(|| anyhow!("storage {k} is too large"))?;
        let mut buf = vec![0u8; size];
        r.read_exact(&mut buf)
            .with_context(|| format!("reading storage {k}"))?;
        storages.insert(k, buf);
    }
    let mut out = HashMap::new();
    for info in infos {
        let key = info.path.trim_start_matches('/');
        let data = storages
            .get(key)
            .ok_or_else(|| anyhow!("{}: storage {key} missing", info.name))?;
        if !info.layout.is_contiguous() {
            bail!("{}: not a contiguous tensor", info.name);
        }
        let start = info.layout.start_offset();
        let len = info.layout.shape().elem_count() * info.dtype.size_in_bytes();
        let bytes = data
            .get(start..start + len)
            .ok_or_else(|| anyhow!("{}: past the end of its storage", info.name))?;
        let t = Tensor::from_raw_buffer(bytes, info.dtype, info.layout.shape().dims(), device)?;
        out.insert(info.name, t);
    }
    Ok(out)
}
