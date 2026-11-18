use std::collections::VecDeque;

use betterlite_bedrock::sagemaker_embedding::cohere::{self, CohereReplyError, CohereRequestError};
use betterlite_bedrock::sagemaker_embedding::hf::{self, HfReplyError};
use betterlite_bedrock::sagemaker_embedding::{self, EmbeddingConfig};
use betterlite_bedrock::sagemaker_stream::events::EventState;
use betterlite_bedrock::sagemaker_stream::{
    ErrorPolicy, SagemakerChunk, SagemakerEvents, SagemakerStreamDecoder,
};
use betterlite_http::error::HttpError;
use betterlite_openai::cohere_embedding::TextEncoding;
use betterlite_openai::cohere_legacy_embedding::decode::LegacyEmbeddingRecord;
use betterlite_openai::sagemaker_completion::error::SagemakerError;
use betterlite_tokenizer::engine::error::EngineError;
use serde_json::{Value, json};
use thiserror::Error;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::input::{
    CohereReply, CohereRequest, DecodeStream, Headers, HfReply, Input, ModelConfig,
};

pub(crate) fn run(input: &Input) -> HelperResult {
    match input {
        Input::ModelConfig(case) => Ok(model_config(case)),
        Input::Headers(case) => Ok(headers(case)),
        Input::HfRequest(case) => {
            Ok(json!({ "request": hf::transform_request(&case.input, &case.optional_params) }))
        }
        Input::HfReply(case) => hf_reply(case),
        Input::CohereMap(case) => Ok(json!({
            "mapped": cohere::map_openai_params(&case.non_default, case.optional_params.clone())
        })),
        Input::CohereRequest(case) => cohere_request(case),
        Input::CohereReply(case) => cohere_reply(case),
        Input::DecodeStream(case) => decode_stream(case),
    }
}
fn model_config(case: &ModelConfig) -> Value {
    let config = match sagemaker_embedding::get_model_config(&case.model) {
        EmbeddingConfig::Voyage => "voyage",
        EmbeddingConfig::Cohere => "cohere",
        EmbeddingConfig::HuggingFace => "hugging_face",
    };
    json!({
        "config": config,
        "supported": sagemaker_embedding::supported_openai_params(&case.model),
    })
}
// both LiteLLM methods return only the content type, whatever headers the caller brought
fn headers(case: &Headers) -> Value {
    json!({
        "kind": case.kind,
        "headers": sagemaker_embedding::validate_environment(),
        "given": case.headers,
    })
}

// the HF response serializes to exactly the shape the helper answers
fn hf_reply(case: &HfReply) -> HelperResult {
    let body = case.body.as_bytes();
    let reply = hf::transform_response(body, case.status, &case.model, &case.request);
    let response = reply.map_err(Failure::from)?;
    Ok(json!(response))
}

fn cohere_request(case: &CohereRequest) -> HelperResult {
    let request = cohere::transform_request(&case.model, &case.input, &case.optional_params)
        .map_err(Failure::from)?;
    Ok(json!({ "request": request }))
}

fn cohere_reply(case: &CohereReply) -> HelperResult {
    let response = cohere::transform_response(
        case.body.as_bytes(),
        &case.model,
        case.input.as_ref(),
        &case.request_data,
        &Chars,
    )
    .map_err(Failure::from)?;
    Ok(json!({
        "object": response.object,
        "data": response.data.iter().map(cohere_row).collect::<Vec<_>>(),
        "model": response.model,
        "usage": {
            "prompt_tokens": response.usage.prompt_tokens,
            "completion_tokens": response.usage.completion_tokens,
            "total_tokens": response.usage.total_tokens,
        },
    }))
}

// the replied rows carry a type only when the answer groups them by embedding type
fn cohere_row(row: &LegacyEmbeddingRecord) -> Value {
    let mut value = json!({ "object": row.object, "index": row.index, "embedding": row.embedding });
    if let (Some(kind), Value::Object(fields)) = (&row.kind, &mut value) {
        fields.insert(String::from("type"), json!(kind));
    }
    value
}

fn decode_stream(case: &DecodeStream) -> HelperResult {
    let mut chunks: Vec<Result<Vec<u8>, HttpError>> = Vec::with_capacity(case.chunks.len());
    for chunk in &case.chunks {
        chunks.push(Ok(hex::decode(chunk).map_err(Failure::from)?));
    }
    let policy = if case.sync {
        ErrorPolicy::Propagate
    } else {
        ErrorPolicy::Skip
    };
    let decoder = SagemakerStreamDecoder {
        model: case.model.clone(),
        is_messages_api: case.is_messages_api,
        policy,
    };
    let events = SagemakerEvents {
        state: EventState {
            model: decoder.model,
            is_messages_api: decoder.is_messages_api,
            policy: decoder.policy,
            frames: bytes::BytesMut::new(),
            accumulated: String::new(),
        },
        chunks: chunks.into_iter(),
        pending: VecDeque::new(),
        done: false,
    };
    let mut items = Vec::new();
    let mut failure = Value::Null;
    for item in events {
        match item {
            Ok(Some(chunk)) => items.push(chunk_view(&chunk)),
            Ok(None) => items.push(json!({ "kind": "none" })),
            Err(_error) => {
                failure = json!({ "status": Value::Null });
                break;
            }
        }
    }
    Ok(json!({ "items": items, "failure": failure }))
}

fn chunk_view(chunk: &SagemakerChunk) -> Value {
    match chunk {
        SagemakerChunk::Chat(chat) => json!({
            "kind": "stream",
            "id": chat.id,
            "created": chat.created,
            "model": chat.model,
            "object": chat.object,
            "choices": chat.choices.iter().map(|choice| json!({
                "index": choice.index,
                "finish_reason": choice.finish_reason,
                "content": choice.delta.content,
                "role": choice.delta.role,
                "tool_calls": choice.delta.tool_calls.as_ref().map(|calls| calls.iter().map(|call| json!({
                    "index": call.index,
                    "id": call.id,
                    "type": call.kind,
                    "function": call.function.as_ref().map(|function| json!({
                        "name": function.name,
                        "arguments": function.arguments,
                    })),
                })).collect::<Vec<_>>()),
            })).collect::<Vec<_>>(),
        }),
        SagemakerChunk::Token(token) => json!({
            "kind": "generic",
            "text": token.text,
            "is_finished": token.is_finished,
            "finish_reason": token.finish_reason,
            "usage": token.usage,
            "index": token.index,
        }),
    }
}

// LiteLLM's usage path encodes every input text; the helper counts characters like a tokenizer of one id each
struct Chars;

impl TextEncoding for Chars {
    fn count_tokens(&self, text: &str) -> Result<usize, EngineError> {
        Ok(text.chars().count())
    }
}

#[derive(Debug, Error)]
pub(crate) enum Failure {
    #[error(transparent)]
    Sagemaker(#[from] SagemakerError),
    #[error(transparent)]
    HfReply(#[from] HfReplyError),
    #[error(transparent)]
    CohereRequest(#[from] CohereRequestError),
    #[error(transparent)]
    CohereReply(#[from] CohereReplyError),
    #[error("the stream chunk is not hex: {source}")]
    Hex {
        #[from]
        source: hex::FromHexError,
    },
}

impl From<Failure> for Box<HelperError> {
    fn from(failure: Failure) -> Self {
        let status = match &failure {
            Failure::Sagemaker(error) => Some(error.status_code),
            Failure::HfReply(error) => Some(error.status_code()),
            Failure::CohereRequest(_) | Failure::CohereReply(_) | Failure::Hex { .. } => None,
        };
        Self::new(HelperError {
            class: error_class(&failure).to_owned(),
            status,
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
            message: failure.to_string(),
        })
    }
}

// only the status crosses the comparison, so the name just says which LiteLLM class raised
const fn error_class(failure: &Failure) -> &'static str {
    match failure {
        Failure::Sagemaker(_) | Failure::HfReply(_) => "SagemakerError",
        Failure::CohereRequest(_) | Failure::CohereReply(_) | Failure::Hex { .. } => "ValueError",
    }
}
