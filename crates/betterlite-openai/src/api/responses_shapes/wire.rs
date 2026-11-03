use serde::Deserialize;
use serde::de::{DeserializeOwned, IgnoredAny};
use serde_json::value::RawValue;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireDecodeLimits {
    pub max_json_bytes: usize,
    pub max_event_bytes: usize,
    pub max_items: usize,
}

#[derive(Debug, Error)]
pub enum ShapeDecodeError {
    #[error("JSON exceeds byte limit")]
    JsonTooLarge,
    #[error("JSON item limit exceeded")]
    TooManyItems,
    #[error("JSON nesting limit exceeded")]
    TooDeep,
    #[error("invalid Responses record: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn decode_responses_shapes<T: DeserializeOwned>(
    bytes: &[u8],
    limits: WireDecodeLimits,
) -> Result<T, ShapeDecodeError> {
    if bytes.len() > limits.max_json_bytes || bytes.len() > limits.max_event_bytes {
        return Err(ShapeDecodeError::JsonTooLarge);
    }
    let root: &RawValue = serde_json::from_slice(bytes)?;
    let mut pending = vec![(root, 0_usize)];
    let mut remaining = limits.max_items;
    while let Some((value, depth)) = pending.pop() {
        if depth > 128 {
            return Err(ShapeDecodeError::TooDeep);
        }
        let children = match value.get().as_bytes().first() {
            Some(b'[') => serde_json::from_str::<Vec<&RawValue>>(value.get())?,
            Some(b'{') => serde_json::from_str::<Members<'_>>(value.get())?
                .0
                .into_iter()
                .map(|(_, child)| child)
                .collect(),
            _ => Vec::new(),
        };
        remaining = remaining
            .checked_sub(children.len())
            .ok_or(ShapeDecodeError::TooManyItems)?;
        pending.extend(
            children
                .into_iter()
                .map(|child| (child, depth.saturating_add(1))),
        );
    }
    Ok(serde_json::from_slice(bytes)?)
}

#[serde_with::serde_as]
#[derive(Deserialize)]
#[serde(transparent)]
struct Members<'input>(
    #[serde_as(as = "serde_with::Map<_, _>")]
    #[serde(borrow)]
    Vec<(IgnoredAny, &'input RawValue)>,
);
