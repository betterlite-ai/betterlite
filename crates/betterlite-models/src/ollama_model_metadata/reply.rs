use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::value::RawValue;

const CONTEXT_LENGTH_MARKER: &str = "context_length";
pub(super) const MAX_REPLY_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ShowReply {
    pub template: Option<String>,
    pub context_length: Option<i64>,
}

#[derive(Debug, thiserror::Error)]
pub enum OllamaMetadataError {
    #[error("ollama /api/show reply is {size} bytes, over the {limit} byte limit")]
    ReplyTooLarge { size: usize, limit: usize },
    #[error("ollama /api/show reply at {path}: {source}")]
    Reply {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("ollama /api/show reply field {field}: {source}")]
    ReplyField {
        field: String,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Deserialize)]
struct RawReply {
    #[serde(default)]
    template: Option<String>,
    #[serde(default)]
    model_info: IndexMap<String, Box<RawValue>>,
}

pub fn decode_ollama_model_reply(bytes: &[u8]) -> Result<ShowReply, OllamaMetadataError> {
    if bytes.len() > MAX_REPLY_BYTES {
        return Err(OllamaMetadataError::ReplyTooLarge {
            size: bytes.len(),
            limit: MAX_REPLY_BYTES,
        });
    }
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let raw: RawReply = serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
        OllamaMetadataError::Reply {
            path: error.path().to_string(),
            source: error.into_inner(),
        }
    })?;
    let first_context_length = raw
        .model_info
        .iter()
        .find(|(key, _)| key.contains(CONTEXT_LENGTH_MARKER));
    let context_length = match first_context_length {
        Some((key, value)) => {
            serde_json::from_str::<Option<i64>>(value.get()).map_err(|source| {
                OllamaMetadataError::ReplyField {
                    field: format!("model_info.{key}"),
                    source,
                }
            })?
        }
        None => None,
    };
    Ok(ShowReply {
        template: raw.template,
        context_length,
    })
}
