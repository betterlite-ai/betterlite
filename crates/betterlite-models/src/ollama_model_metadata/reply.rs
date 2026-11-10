use indexmap::IndexMap;
use serde_json::value::RawValue;

const CONTEXT_LENGTH_MARKER: &str = "context_length";
const TEMPLATE_FIELD: &str = "template";
const MODEL_INFO_FIELD: &str = "model_info";
pub(super) const MAX_REPLY_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ShowReply {
    pub template: String,
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

// a struct derive would also accept a top-level array, which the source rejects on its first .get
pub fn decode_ollama_model_reply(bytes: &[u8]) -> Result<ShowReply, OllamaMetadataError> {
    if bytes.len() > MAX_REPLY_BYTES {
        return Err(OllamaMetadataError::ReplyTooLarge {
            size: bytes.len(),
            limit: MAX_REPLY_BYTES,
        });
    }
    let raw = decode_object(bytes)?;
    Ok(ShowReply {
        template: decode_template(&raw)?.unwrap_or_default(),
        context_length: decode_context_length(&raw)?,
    })
}

fn decode_object(bytes: &[u8]) -> Result<IndexMap<String, Box<RawValue>>, OllamaMetadataError> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
        OllamaMetadataError::Reply {
            path: error.path().to_string(),
            source: error.into_inner(),
        }
    })
}

fn decode_template(
    raw: &IndexMap<String, Box<RawValue>>,
) -> Result<Option<String>, OllamaMetadataError> {
    raw.get(TEMPLATE_FIELD).map_or(Ok(None), |value| {
        serde_json::from_str::<Option<String>>(value.get()).map_err(|source| {
            OllamaMetadataError::ReplyField {
                field: TEMPLATE_FIELD.to_owned(),
                source,
            }
        })
    })
}

fn decode_context_length(
    raw: &IndexMap<String, Box<RawValue>>,
) -> Result<Option<i64>, OllamaMetadataError> {
    let model_info: IndexMap<String, Box<RawValue>> = match raw.get(MODEL_INFO_FIELD) {
        Some(value) => {
            serde_json::from_str(value.get()).map_err(|source| OllamaMetadataError::ReplyField {
                field: MODEL_INFO_FIELD.to_owned(),
                source,
            })?
        }
        None => IndexMap::new(),
    };
    let first_context_length = model_info
        .iter()
        .find(|(key, _)| key.contains(CONTEXT_LENGTH_MARKER));
    match first_context_length {
        Some((key, value)) => serde_json::from_str::<Option<i64>>(value.get()).map_err(|source| {
            OllamaMetadataError::ReplyField {
                field: format!("{MODEL_INFO_FIELD}.{key}"),
                source,
            }
        }),
        None => Ok(None),
    }
}
