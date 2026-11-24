use std::mem::take;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_core::api::media_relay_logging::spend::decimal_json;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::cohere_embedding::usage::CohereEmbeddingUsage;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Serialize;
use thiserror::Error;

use super::base_passthrough_logging::{
    PassthroughChatInput, PassthroughChatLogger, PassthroughLoggingError, PassthroughOutcome,
    PassthroughPayload, PassthroughStage, request_model,
};
use super::response_payload::ResponseLoggingKwargs;

pub mod chat;
pub mod embedding;
mod stream;

pub(crate) mod tests;

pub(crate) const COHERE: &str = "cohere";

// The failures this module adds to the handler's error; the base enum carries them as a source.
#[derive(Debug, Error)]
pub(crate) enum CohereLoggingError {
    #[error("the bundled cl100k tokenizer could not be built")]
    Tokenizer,
    #[error("the Cohere embedding {field} value must be a list")]
    EmbeddingList { field: &'static str },
    #[error("the Cohere embedding input carries a value that is not a string")]
    EmbeddingText,
}

// LiteLLM's EmbeddingResponse as it is logged, with the cost in the private _hidden_params.
#[derive(Debug, Clone, Serialize)]
pub struct EmbeddingResponse {
    pub object: String,
    pub data: Vec<Json>,
    pub model: String,
    pub usage: CohereEmbeddingUsage,
    #[serde(rename = "_hidden_params")]
    pub hidden_params: JsonObject,
}

pub struct EmbeddingDecodeInput<'input> {
    pub upstream_body: &'input [u8],
    pub request_body: &'input JsonObject,
    pub model: &'input str,
    pub input: &'input [Json],
}

pub struct StandardObjectInput<'input> {
    pub kwargs: &'input ResponseLoggingKwargs,
    pub response: &'input EmbeddingResponse,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
}

// The three calls the embed branch makes, so a test can stand in for B1418, B068 and B190.
pub trait CohereEmbeddingTools: Send + Sync {
    fn decode_embedding(
        &self,
        input: EmbeddingDecodeInput<'_>,
        call_log: &mut LoggingState,
    ) -> Result<EmbeddingResponse, PassthroughLoggingError>;

    fn embedding_cost(
        &self,
        response: &EmbeddingResponse,
        model: &str,
    ) -> Result<Decimal, PassthroughLoggingError>;

    fn standard_logging_object(
        &self,
        input: StandardObjectInput<'_>,
        call_log: &mut LoggingState,
    ) -> Result<(), PassthroughLoggingError>;
}

pub struct CoherePassthroughLogger {
    chat: PassthroughChatLogger,
    embedding: Arc<dyn CohereEmbeddingTools>,
}

impl CoherePassthroughLogger {
    #[must_use]
    // ast-grep-ignore: no-one-line-functions - B3606 and B8931 build the Cohere logger through this fixed constructor
    pub fn new(chat: PassthroughChatLogger, embedding: Arc<dyn CohereEmbeddingTools>) -> Self {
        Self { chat, embedding }
    }

    pub fn cohere_passthrough_handler(
        &self,
        mut input: PassthroughChatInput<'_>,
    ) -> Result<PassthroughOutcome, PassthroughLoggingError> {
        if input.url_route.contains("/v1/embed")
            && !input.url_route.contains("/v1/embeddings")
            && let Ok(outcome) = self.embed_branch(&mut input)
        {
            return Ok(outcome);
        }
        self.chat.passthrough_chat_handler(input)
    }

    // Every step is inside the catch: LiteLLM's blanket except turns any failure into the chat fallback.
    fn embed_branch(
        &self,
        input: &mut PassthroughChatInput<'_>,
    ) -> Result<PassthroughOutcome, PassthroughLoggingError> {
        let model = request_model(input.request_body, Some(input.response_body))?;
        let texts = embedding_texts(input.request_body)?;
        let mut response = self.embedding.decode_embedding(
            EmbeddingDecodeInput {
                upstream_body: input.upstream.body,
                request_body: input.request_body,
                model: &model,
                input: &texts,
            },
            &mut *input.call_log,
        )?;
        let cost = self.embedding.embedding_cost(&response, &model)?;
        response
            .hidden_params
            .insert("response_cost".to_owned(), decimal_json(cost));
        apply_embed_kwargs(&mut input.kwargs, self.chat.payload.as_ref(), &model, cost);
        self.embedding.standard_logging_object(
            StandardObjectInput {
                kwargs: &input.kwargs,
                response: &response,
                start_time: input.start_time,
                end_time: input.end_time,
            },
            &mut *input.call_log,
        )?;
        write_call_log(&mut *input.call_log, &model, cost);
        let response_json = serde_json::to_string(&response)
            .map_err(|source| PassthroughStage::Payload.failed(source))?;
        let result: Json = serde_json::from_str(&response_json)
            .map_err(|source| PassthroughStage::Payload.failed(source))?;
        Ok(PassthroughOutcome {
            result: Some(PassThroughResponse::Embedding(result)),
            kwargs: take(&mut input.kwargs),
        })
    }
}

// kwargs["response_cost"], ["model"] and ["custom_llm_provider"], then setdefault plus update of proxy_server_request.
fn apply_embed_kwargs(
    kwargs: &mut ResponseLoggingKwargs,
    payload: &dyn PassthroughPayload,
    model: &str,
    cost: Decimal,
) {
    kwargs.response_cost = Some(cost);
    kwargs.model = Some(model.to_owned());
    let Some(body) = kwargs.passthrough_request_body.as_ref() else {
        return;
    };
    let Some(user) = payload.end_user(Some(body)) else {
        return;
    };
    let mut user_field = IndexMap::new();
    user_field.insert("user".to_owned(), Json::Text(user));
    let mut request = IndexMap::new();
    request.insert("body".to_owned(), Json::Object(user_field));
    kwargs
        .litellm_params
        .insert("proxy_server_request".to_owned(), Json::Object(request));
}

// logging_obj.model_call_details["model"], ["custom_llm_provider"] and ["response_cost"].
fn write_call_log(call_log: &mut LoggingState, model: &str, cost: Decimal) {
    call_log
        .model_call_details
        .insert("model".to_owned(), Json::Text(model.to_owned()));
    call_log.model_call_details.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(COHERE.to_owned()),
    );
    call_log
        .model_call_details
        .insert("response_cost".to_owned(), decimal_json(cost));
}

// request_body["texts"] wins only when it is a non-empty list; otherwise request_body["input"], default empty.
fn embedding_texts(request_body: &JsonObject) -> Result<Vec<Json>, PassthroughLoggingError> {
    let fallback = match request_body.get("texts") {
        Some(Json::List(items)) if !items.is_empty() => return Ok(items.clone()),
        None | Some(Json::Null | Json::List(_)) => request_body.get("input"),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            return Err(PassthroughStage::Transform
                .failed(CohereLoggingError::EmbeddingList { field: "texts" }));
        }
    };
    match fallback {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::List(items)) => Ok(items.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Err(PassthroughStage::Transform
                .failed(CohereLoggingError::EmbeddingList { field: "input" }))
        }
    }
}

// The clock the V2 chat transform and the stream builder stamp their responses with.
pub(crate) fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}
