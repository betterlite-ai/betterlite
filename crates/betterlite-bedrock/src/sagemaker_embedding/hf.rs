use betterlite_openai::embedding::{EmbeddingInput, EmbeddingVector};
use betterlite_openai::sagemaker_completion::error::SagemakerError;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;
use serde::Serialize;
use serde_json::Number;
use thiserror::Error;

const EMBEDDING_OBJECT: &str = "embedding";
const RESPONSE_OBJECT: &str = "list";
const UNEXPECTED_FORMAT: u16 = 500;
const NOT_A_LIST_STATUS: u16 = 422;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HfEmbeddingResponse {
    pub object: &'static str,
    pub data: Vec<HfEmbeddingRecord>,
    pub model: String,
    pub usage: HfUsage,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HfEmbeddingRecord {
    pub object: &'static str,
    pub index: usize,
    pub embedding: EmbeddingVector,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct HfUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Error)]
pub enum HfReplyError {
    #[error("Failed to parse response: {source}")]
    Parse {
        status_code: u16,
        #[source]
        source: serde_json::Error,
    },
    #[error("the HF reply does not have the declared response shape")]
    Shape {
        #[source]
        source: serde_json::Error,
    },
    #[error("Unexpected response format. Expected list or dict with 'embedding' key, got: {kind}")]
    Unexpected { kind: &'static str },
    #[error("HF response not in expected format - {value}")]
    NotAList { value: String },
    #[error("the HF request inputs are neither a text nor a list of texts")]
    Inputs {
        #[source]
        source: serde_json::Error,
    },
    #[error("the HF input word count exceeds the supported range")]
    Overflow,
}

// the declared response: TEI's raw list, a dict under "embedding", or a value that only names its type
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum HfReplyBody {
    Embeddings(Vec<EmbeddingVector>),
    Wrapped(HfWrappedReply),
    Scalar(HfScalar),
}

#[derive(Debug, Deserialize)]
struct HfWrappedReply {
    #[serde(default)]
    embedding: HfEmbeddingValue,
}

// a missing key names the dict, a present value that is not a row list is read as its JSON text
#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum HfEmbeddingValue {
    Rows(Vec<EmbeddingVector>),
    Other(Json),
    #[default]
    Absent,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum HfScalar {
    Bool(bool),
    Number(Number),
    Text(String),
    Null,
}

impl HfReplyError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Parse { status_code, .. } => *status_code,
            Self::NotAList { .. } => NOT_A_LIST_STATUS,
            Self::Shape { .. } | Self::Unexpected { .. } | Self::Inputs { .. } | Self::Overflow => {
                UNEXPECTED_FORMAT
            }
        }
    }
}

impl From<HfReplyError> for SagemakerError {
    fn from(error: HfReplyError) -> Self {
        Self {
            status_code: error.status_code(),
            message: error.to_string(),
            headers: None,
        }
    }
}

// transform_embedding_request: the caller's options are the body around the input, and an "inputs" option wins
#[must_use]
pub fn transform_request(input: &EmbeddingInput, optional_params: &Object) -> Object {
    let mut body = Object::from([(String::from("inputs"), input_json(input))]);
    body.extend(optional_params.clone());
    body
}

// transform_embedding_response: a list is the embeddings, a dict carries them under "embedding"
pub fn transform_response(
    body: &[u8],
    status_code: u16,
    model: &str,
    request: &Object,
) -> Result<HfEmbeddingResponse, HfReplyError> {
    let embeddings = reply_embeddings(decode_reply(body, status_code)?)?;
    let prompt_tokens = count_input_words(request)?;
    Ok(HfEmbeddingResponse {
        object: RESPONSE_OBJECT,
        data: embedding_rows(&embeddings),
        model: model.to_owned(),
        usage: HfUsage {
            prompt_tokens,
            completion_tokens: 0,
            total_tokens: prompt_tokens,
        },
    })
}

fn decode_reply(body: &[u8], status_code: u16) -> Result<HfReplyBody, HfReplyError> {
    serde_json::from_slice(body).map_err(|source| {
        if source.is_data() {
            HfReplyError::Shape { source }
        } else {
            HfReplyError::Parse {
                status_code,
                source,
            }
        }
    })
}

fn reply_embeddings(body: HfReplyBody) -> Result<Vec<EmbeddingVector>, HfReplyError> {
    match body {
        HfReplyBody::Embeddings(rows) => Ok(rows),
        HfReplyBody::Wrapped(wrapped) => match wrapped.embedding {
            HfEmbeddingValue::Absent => Err(HfReplyError::Unexpected { kind: "dict" }),
            HfEmbeddingValue::Rows(rows) => Ok(rows),
            HfEmbeddingValue::Other(value) => {
                let rendered = serde_json::to_string(&value)
                    .map_err(|source| HfReplyError::Shape { source })?;
                Err(HfReplyError::NotAList { value: rendered })
            }
        },
        HfReplyBody::Scalar(scalar) => Err(HfReplyError::Unexpected {
            kind: scalar.kind(),
        }),
    }
}

fn embedding_rows(embeddings: &[EmbeddingVector]) -> Vec<HfEmbeddingRecord> {
    embeddings
        .iter()
        .enumerate()
        .map(|(index, embedding)| HfEmbeddingRecord {
            object: EMBEDDING_OBJECT,
            index,
            embedding: embedding.clone(),
        })
        .collect()
}

impl HfScalar {
    fn kind(&self) -> &'static str {
        match self {
            Self::Bool(_value) => "bool",
            Self::Number(number) => number_kind(number),
            Self::Text(_value) => "str",
            Self::Null => "NoneType",
        }
    }
}

// LiteLLM names the value with type(value).__name__
fn number_kind(number: &Number) -> &'static str {
    if number.is_i64() || number.is_u64() {
        "int"
    } else {
        "float"
    }
}

fn input_json(input: &EmbeddingInput) -> Json {
    match input {
        EmbeddingInput::Text(text) => Json::Text(text.clone()),
        EmbeddingInput::Texts(texts) => Json::List(texts.iter().cloned().map(Json::Text).collect()),
        EmbeddingInput::Tokens(tokens) => Json::List(
            tokens
                .iter()
                .map(|token| Json::Number(Number::from(*token)))
                .collect(),
        ),
        EmbeddingInput::TokenLists(lists) => Json::List(
            lists
                .iter()
                .map(|tokens| {
                    Json::List(
                        tokens
                            .iter()
                            .map(|token| Json::Number(Number::from(*token)))
                            .collect(),
                    )
                })
                .collect(),
        ),
    }
}

// request_data["inputs"]: a text counts its non-space characters, as Python walks the string it was given
fn count_input_words(request: &Object) -> Result<u64, HfReplyError> {
    let Some(inputs) = request.get("inputs") else {
        return Ok(0);
    };
    if matches!(inputs, Json::Null) {
        return Ok(0);
    }
    let value = serde_json::to_value(inputs).map_err(|source| HfReplyError::Shape { source })?;
    let input = serde_json::from_value::<EmbeddingInput>(value)
        .map_err(|source| HfReplyError::Inputs { source })?;
    let words = match input {
        EmbeddingInput::Text(text) => text.chars().filter(|item| !item.is_whitespace()).count(),
        EmbeddingInput::Texts(texts) => texts
            .iter()
            .map(|text| text.split_whitespace().count())
            .sum(),
        EmbeddingInput::Tokens(_) | EmbeddingInput::TokenLists(_) => 0,
    };
    u64::try_from(words).map_err(|_overflow| HfReplyError::Overflow)
}
mod tests;
