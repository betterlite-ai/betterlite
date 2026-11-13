use serde::Deserialize;
use serde::de::Error as _;

use super::error::CacheCreateError;

// the pooled transport caps a reply at 16 MiB; a decoder handed more than that refuses it
pub const CREATED_CACHE_LIMIT: usize = 0x0100_0000;

// LiteLLM's VertexAICachedContentResponseObject declares name and model; only the name reaches the request, and a missing one stays absent rather than falling back to displayName
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct CreatedCache {
    #[serde(default)]
    pub name: Option<String>,
}

pub fn decode_created_cache(bytes: &[u8]) -> Result<CreatedCache, CacheCreateError> {
    if bytes.len() > CREATED_CACHE_LIMIT {
        return Err(CacheCreateError::TooLarge {
            limit: CREATED_CACHE_LIMIT,
        });
    }
    // serde accepts a sequence for a struct, so the root type is checked before the typed parse
    if bytes.iter().find(|byte| !byte.is_ascii_whitespace()) != Some(&b'{') {
        return Err(CacheCreateError::Decode {
            path: "the document root".to_owned(),
            source: serde_json::Error::custom("the value is not a JSON object"),
        });
    }
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let cache: CreatedCache =
        serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
            CacheCreateError::Decode {
                path: error.path().to_string(),
                source: error.into_inner(),
            }
        })?;
    deserializer
        .end()
        .map_err(|source| CacheCreateError::Decode {
            path: "the document root".to_owned(),
            source,
        })?;
    Ok(cache)
}
