//! Replay encoding and config hashing. See `docs/REPLAY_FORMAT.md`.

use mm_core::Replay;
use serde::Serialize;

use crate::rng::fnv1a;

#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    #[error("msgpack encode: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
    #[error("msgpack decode: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// MessagePack with named fields (maps, not arrays). Byte-identical for equal replays.
pub fn encode_msgpack(r: &Replay) -> Result<Vec<u8>, ReplayError> {
    Ok(rmp_serde::to_vec_named(r)?)
}

pub fn decode_msgpack(bytes: &[u8]) -> Result<Replay, ReplayError> {
    Ok(rmp_serde::from_slice(bytes)?)
}

pub fn to_json(r: &Replay) -> Result<String, ReplayError> {
    Ok(serde_json::to_string(r)?)
}

/// Stable 64-bit FNV-1a (hex, 16 chars) over canonical JSON bytes (keys sorted).
pub fn config_hash<T: Serialize>(v: &T) -> String {
    let bytes = serde_json::to_value(v)
        .and_then(|v| serde_json::to_vec(&v))
        .unwrap_or_default();
    format!("{:016x}", fnv1a(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_and_order_independent() {
        let a = serde_json::json!({"b": 1, "a": [1, 2]});
        let b = serde_json::json!({"a": [1, 2], "b": 1});
        assert_eq!(config_hash(&a), config_hash(&b));
        assert_ne!(config_hash(&a), config_hash(&serde_json::json!({"a": 1})));
        // FNV-1a("") offset basis sanity: hash of the JSON `null` is fixed.
        assert_eq!(config_hash(&serde_json::Value::Null).len(), 16);
    }
}
