use betterlite_openai::sagemaker_completion::error::SagemakerError;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;

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
    pub embedding: Json,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct HfUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

// transform_embedding_request: the caller's options are the body around the input, and an "inputs" option wins
#[must_use]
pub fn transform_request(input: &Json, optional_params: &Object) -> Object {
    let mut body = Object::from([(String::from("inputs"), input.clone())]);
    body.extend(optional_params.clone());
    body
}

// transform_embedding_response: a list is the embeddings, a dict carries them under "embedding"
pub fn transform_response(
    body: &[u8],
    status_code: u16,
    model: &str,
    request: &Object,
) -> Result<HfEmbeddingResponse, SagemakerError> {
    let decoded = decode_reply(body, status_code)?;
    let embeddings = reply_embeddings(&decoded)?;
    let prompt_tokens = count_input_words(request.get("inputs"))?;
    Ok(HfEmbeddingResponse {
        object: RESPONSE_OBJECT,
        data: embedding_rows(embeddings),
        model: model.to_owned(),
        usage: HfUsage {
            prompt_tokens,
            completion_tokens: 0,
            total_tokens: prompt_tokens,
        },
    })
}

fn decode_reply(body: &[u8], status_code: u16) -> Result<Json, SagemakerError> {
    serde_json::from_slice(body).map_err(|source| SagemakerError {
        status_code,
        message: format!("Failed to parse response: {source}"),
        headers: None,
    })
}

fn reply_embeddings(decoded: &Json) -> Result<&[Json], SagemakerError> {
    let candidate = match decoded {
        Json::List(_) => decoded,
        Json::Object(fields) => fields.get("embedding").ok_or_else(|| unexpected(decoded))?,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {
            return Err(unexpected(decoded));
        }
    };
    let Json::List(embeddings) = candidate else {
        let rendered = serde_json::to_string(candidate).map_err(|source| SagemakerError {
            status_code: UNEXPECTED_FORMAT,
            message: format!("the HF reply value cannot be read: {source}"),
            headers: None,
        })?;
        return Err(SagemakerError {
            status_code: NOT_A_LIST_STATUS,
            message: format!("HF response not in expected format - {rendered}"),
            headers: None,
        });
    };
    Ok(embeddings)
}

fn embedding_rows(embeddings: &[Json]) -> Vec<HfEmbeddingRecord> {
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

fn unexpected(value: &Json) -> SagemakerError {
    SagemakerError {
        status_code: UNEXPECTED_FORMAT,
        message: format!(
            "Unexpected response format. Expected list or dict with 'embedding' key, got: {}",
            python_type_name(value)
        ),
        headers: None,
    }
}

fn python_type_name(value: &Json) -> &'static str {
    match value {
        Json::Null => "NoneType",
        Json::Bool(_) => "bool",
        Json::Number(number) if number.is_i64() || number.is_u64() => "int",
        Json::Number(_) => "float",
        Json::Text(_) => "str",
        Json::List(_) => "list",
        Json::Object(_) => "dict",
    }
}

// request_data["inputs"]: a text counts its non-space characters, as Python walks the string it was given
fn count_input_words(inputs: Option<&Json>) -> Result<u64, SagemakerError> {
    let words = match inputs {
        None | Some(Json::Null) => 0,
        Some(Json::Text(text)) => text.chars().filter(|item| !item.is_whitespace()).count(),
        Some(Json::List(items)) => items
            .iter()
            .filter_map(|item| item.as_str())
            .map(|text| text.split_whitespace().count())
            .sum(),
        Some(Json::Bool(_) | Json::Number(_) | Json::Object(_)) => {
            return Err(SagemakerError {
                status_code: UNEXPECTED_FORMAT,
                message: String::from("the request inputs is neither a text nor a list of texts"),
                headers: None,
            });
        }
    };
    u64::try_from(words).map_err(|_overflow| SagemakerError {
        status_code: UNEXPECTED_FORMAT,
        message: String::from("the HF input word count exceeds the supported range"),
        headers: None,
    })
}

mod tests;
