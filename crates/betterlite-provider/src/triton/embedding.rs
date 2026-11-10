use betterlite_openai::embedding::{
    EmbeddingInput, EmbeddingRecord, EmbeddingUsage, EmbeddingVector,
};
use betterlite_prompts::builtin::Field;
use http::HeaderMap;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Number;
use serde_json::value::RawValue;

use super::datatype::TritonDatatype;
use super::error::TritonError;
use super::reply::{decode_reply, parse_object};

pub const INPUT_NAME: &str = "input_text";
pub const SUPPORTED_PARAMS: [&str; 0] = [];

pub trait EmbeddingConfig {
    fn validate_environment(&self) -> HeaderMap;

    fn map_openai_params(
        &self,
        optional_params: IndexMap<String, Field>,
    ) -> IndexMap<String, Field>;
}

#[derive(Debug, Clone, Copy)]
pub struct TritonEmbeddingConfig;

// LiteLLM sends the embedding call with no headers at all and keeps every optional param as given
impl EmbeddingConfig for TritonEmbeddingConfig {
    // the empty default is an equivalent mutant: HeaderMap::new() is already the default map
    #[cfg_attr(test, mutants::skip)]
    fn validate_environment(&self) -> HeaderMap {
        HeaderMap::new()
    }

    fn map_openai_params(
        &self,
        optional_params: IndexMap<String, Field>,
    ) -> IndexMap<String, Field> {
        optional_params
    }
}

pub trait TextTokenCounter {
    fn count_text(&self, model: &str, text: &str) -> Option<usize>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddingRequest {
    pub inputs: Vec<EmbeddingInputEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddingInputEntry {
    pub name: String,
    pub shape: [usize; 1],
    pub datatype: TritonDatatype,
    pub data: EmbeddingInput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EmbeddingReply {
    pub model: String,
    pub data: Vec<EmbeddingRecord>,
    pub usage: EmbeddingUsage,
}

#[derive(Debug, Deserialize)]
struct EmbeddingBody {
    outputs: Vec<Box<RawValue>>,
    #[serde(default)]
    model_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingOutput {
    shape: Vec<i64>,
    data: Vec<f64>,
}

#[must_use]
pub fn transform_embedding_request(input: EmbeddingInput) -> EmbeddingRequest {
    let shape = match &input {
        EmbeddingInput::Text(text) => text.chars().count(),
        EmbeddingInput::Texts(values) => values.len(),
        EmbeddingInput::Tokens(values) => values.len(),
        EmbeddingInput::TokenLists(values) => values.len(),
    };
    EmbeddingRequest {
        inputs: vec![EmbeddingInputEntry {
            name: String::from(INPUT_NAME),
            shape: [shape],
            datatype: TritonDatatype::Bytes,
            data: input,
        }],
    }
}

pub fn transform_embedding_response(
    status_code: u16,
    body: &[u8],
    model: &str,
    request: &EmbeddingRequest,
    counter: &dyn TextTokenCounter,
) -> Result<EmbeddingReply, TritonError> {
    let EmbeddingBody {
        outputs,
        model_name,
    } = decode_reply(status_code, body)?;
    Ok(EmbeddingReply {
        model: model_name.unwrap_or_else(|| String::from("None")),
        data: embedding_records(outputs)?,
        usage: build_embedding_usage(model, request, counter)?,
    })
}

fn embedding_records(outputs: Vec<Box<RawValue>>) -> Result<Vec<EmbeddingRecord>, TritonError> {
    let mut data = Vec::new();
    for raw in outputs {
        let output: EmbeddingOutput =
            parse_object(&raw).map_err(|source| TritonError::ReplyShape { source })?;
        for (position, chunk) in split_embedding_by_shape(&output.data, &output.shape)?
            .into_iter()
            .enumerate()
        {
            let index = u32::try_from(position).map_err(|_error| TritonError::TooManyEmbeddings)?;
            let embedding = chunk
                .iter()
                .map(|value| {
                    Number::from_f64(*value).ok_or(TritonError::InvalidEmbedding { value: *value })
                })
                .collect::<Result<Vec<Number>, TritonError>>()?;
            data.push(EmbeddingRecord {
                object: String::from("embedding"),
                embedding: EmbeddingVector::Floats(embedding),
                index,
            });
        }
    }
    Ok(data)
}

pub fn build_embedding_usage(
    model: &str,
    request: &EmbeddingRequest,
    counter: &dyn TextTokenCounter,
) -> Result<EmbeddingUsage, TritonError> {
    let texts: Vec<String> = match request
        .inputs
        .iter()
        .find(|entry| entry.name == INPUT_NAME)
        .map(|entry| &entry.data)
    {
        Some(EmbeddingInput::Texts(values)) => values.clone(),
        Some(EmbeddingInput::Tokens(ids)) => ids.iter().map(ToString::to_string).collect(),
        // Debug of an integer list is the Python list repr that LiteLLM counts
        Some(EmbeddingInput::TokenLists(lists)) => {
            lists.iter().map(|ids| format!("{ids:?}")).collect()
        }
        Some(EmbeddingInput::Text(_)) | None => Vec::new(),
    };
    let prompt_tokens = texts
        .iter()
        .filter(|text| !text.is_empty())
        .try_fold(0_u32, |total, text| {
            let words = counter
                .count_text(model, text)
                .unwrap_or_else(|| text.split_whitespace().count());
            let tokens = u32::try_from(words).ok()?;
            total.checked_add(tokens)
        })
        .ok_or(TritonError::UsageOverflow)?;
    Ok(EmbeddingUsage {
        prompt_tokens,
        completion_tokens: 0,
        completion_tokens_details: None,
        prompt_tokens_details: None,
        total_tokens: prompt_tokens,
    })
}

pub fn split_embedding_by_shape(data: &[f64], shape: &[i64]) -> Result<Vec<Vec<f64>>, TritonError> {
    let [count, width] = shape else {
        return Err(TritonError::ShapeLength);
    };
    let chunk_count =
        usize::try_from(*count).map_err(|_error| TritonError::ShapeValue { value: *count })?;
    let chunk_width =
        usize::try_from(*width).map_err(|_error| TritonError::ShapeValue { value: *width })?;
    if chunk_count > data.len() {
        return Err(TritonError::ShapeExceedsData {
            count: chunk_count,
            elements: data.len(),
        });
    }
    let mut embeddings = Vec::with_capacity(chunk_count);
    for index in 0..chunk_count {
        let start = index
            .checked_mul(chunk_width)
            .ok_or(TritonError::ShapeValue { value: *width })?;
        let end = start.saturating_add(chunk_width).min(data.len());
        let chunk = data.get(start..end).unwrap_or_default();
        embeddings.push(chunk.to_vec());
    }
    Ok(embeddings)
}
