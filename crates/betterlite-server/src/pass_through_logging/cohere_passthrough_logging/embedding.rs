use std::sync::Arc;

use betterlite_core::openai_pricing::CompletionCost;
use betterlite_core::openai_pricing::embedding_cost::embedding_cost as priced_embedding_cost;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, PrePostEnv, TracingSink,
};
use betterlite_logging::logging_pre_post::post_call::{PostCall as LoggedPostCall, post_call};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_success::ports::StandardPayloadBuilder;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::cohere_legacy_embedding::decode::{
    DecodeInput, LegacyEmbeddingResponse, decode_cohere_legacy_embedding,
};
use betterlite_openai::cohere_legacy_embedding::{EmbeddingHooks, PostCall, PreCall};
use betterlite_openai::embedding::EmbeddingResponse as PricedEmbeddingResponse;
use betterlite_prompts::arguments::json::{Json as PromptsJson, Object as PromptsObject};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_tokenizer::engine::Engine;
use chrono::Utc;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::super::base_passthrough_logging::{PassthroughLoggingError, PassthroughStage};
use super::{
    COHERE, CohereEmbeddingTools, CohereLoggingError, EmbeddingDecodeInput, EmbeddingResponse,
    StandardObjectInput,
};

// The production embed branch: B1418's decode, B068's cost and B190's standard object.
pub struct CohereEmbeddingLogging {
    cost: Arc<dyn CompletionCost + Send + Sync>,
    builder: Arc<dyn StandardPayloadBuilder>,
    encoding: Engine,
}

impl CohereEmbeddingLogging {
    #[must_use]
    pub fn new(
        cost: Arc<dyn CompletionCost + Send + Sync>,
        builder: Arc<dyn StandardPayloadBuilder>,
        encoding: Engine,
    ) -> Self {
        Self {
            cost,
            builder,
            encoding,
        }
    }
}

impl CohereEmbeddingTools for CohereEmbeddingLogging {
    fn decode_embedding(
        &self,
        input: EmbeddingDecodeInput<'_>,
        call_log: &mut LoggingState,
    ) -> Result<EmbeddingResponse, PassthroughLoggingError> {
        let request_json = serde_json::to_string(input.request_body)
            .map_err(|source| PassthroughStage::Transform.failed(source))?;
        let request_data: PromptsObject = serde_json::from_str(&request_json)
            .map_err(|source| PassthroughStage::Transform.failed(source))?;
        let texts = embedding_input_texts(input.input)?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
            .map_err(|source| PassthroughStage::Transform.failed(source))?;
        let sink = TracingSink;
        let environment = logging_environment(&redactor, &sink);
        let mut recorder = Recorder::default();
        let decoded = decode_cohere_legacy_embedding(
            DecodeInput {
                body: input.upstream_body,
                model: input.model,
                api_key: None,
                request_data: &request_data,
                input: &texts,
                encoding: &self.encoding,
            },
            &mut recorder,
        );
        // B1418 logs post_call before it fills the response, so the recorded call is replayed whatever the outcome.
        let replayed = replay(recorder.calls, call_log, &environment);
        let reply = decoded.map_err(|source| PassthroughStage::Transform.failed(source))?;
        replayed?;
        logged_embedding(&reply)
    }

    fn embedding_cost(
        &self,
        response: &EmbeddingResponse,
        model: &str,
    ) -> Result<Decimal, PassthroughLoggingError> {
        let response_json = serde_json::to_string(response)
            .map_err(|source| PassthroughStage::Transform.failed(source))?;
        let priced: PricedEmbeddingResponse = serde_json::from_str(&response_json)
            .map_err(|source| PassthroughStage::Transform.failed(source))?;
        Ok(priced_embedding_cost(
            &priced,
            model,
            COHERE,
            self.cost.as_ref(),
        ))
    }

    // get_standard_logging_object_payload with status "success"; the embed branch drops the answer.
    fn standard_logging_object(
        &self,
        input: StandardObjectInput<'_>,
        call_log: &mut LoggingState,
    ) -> Result<(), PassthroughLoggingError> {
        let response_json = serde_json::to_string(input.response)
            .map_err(|source| PassthroughStage::Payload.failed(source))?;
        let response: Json = serde_json::from_str(&response_json)
            .map_err(|source| PassthroughStage::Payload.failed(source))?;
        let _built = self
            .builder
            .build(call_log, &response, input.start_time, input.end_time);
        Ok(())
    }
}

// The debug-logging environment B1418's post_call reads: no content capture, one redactor, one sink.
fn logging_environment<'env>(
    redactor: &'env Redactor,
    sink: &'env TracingSink,
) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: Utc::now().fixed_offset(),
    }
}

// Every input text must be a string; the token counter would otherwise count a non-text value.
fn embedding_input_texts(input: &[Json]) -> Result<Vec<String>, PassthroughLoggingError> {
    let mut texts = Vec::with_capacity(input.len());
    for value in input {
        match value {
            Json::Text(text) => texts.push(text.clone()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                return Err(PassthroughStage::Transform.failed(CohereLoggingError::EmbeddingText));
            }
        }
    }
    Ok(texts)
}

#[derive(Default)]
struct Recorder {
    calls: Vec<LoggedCall>,
}

struct LoggedCall {
    input: Vec<String>,
    body: PromptsObject,
    original_response: PromptsJson,
}

impl EmbeddingHooks for Recorder {
    fn pre_call(&mut self, _call: PreCall<'_>) {}

    fn post_call(&mut self, call: PostCall<'_>) {
        self.calls.push(LoggedCall {
            input: call.input.to_vec(),
            body: call.body.clone(),
            original_response: call.original_response.clone(),
        });
    }
}

// The post_call LiteLLM makes inside _transform_response: input texts, the request data and the parsed reply.
fn replay(
    calls: Vec<LoggedCall>,
    call_log: &mut LoggingState,
    environment: &PrePostEnv<'_>,
) -> Result<(), PassthroughLoggingError> {
    for call in calls {
        let body_json = serde_json::to_string(&call.body)
            .map_err(|source| PassthroughStage::Transform.failed(source))?;
        let complete_input_dict: JsonObject = serde_json::from_str(&body_json)
            .map_err(|source| PassthroughStage::Transform.failed(source))?;
        let reply_json = serde_json::to_string(&call.original_response)
            .map_err(|source| PassthroughStage::Transform.failed(source))?;
        let original_response: Json = serde_json::from_str(&reply_json)
            .map_err(|source| PassthroughStage::Transform.failed(source))?;
        let additional_args = AdditionalArgs {
            api_base: None,
            headers: None,
            complete_input_dict: Some(RawBody::Object(complete_input_dict)),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        };
        let logged = LoggedPostCall {
            original_response,
            input: Json::List(call.input.into_iter().map(Json::Text).collect()),
            additional_args,
        };
        post_call(call_log, &logged, environment);
    }
    Ok(())
}

fn logged_embedding(
    decoded: &LegacyEmbeddingResponse,
) -> Result<EmbeddingResponse, PassthroughLoggingError> {
    let mut data = Vec::with_capacity(decoded.data.len());
    for record in &decoded.data {
        let record_json = serde_json::to_string(record)
            .map_err(|source| PassthroughStage::Transform.failed(source))?;
        let logged: Json = serde_json::from_str(&record_json)
            .map_err(|source| PassthroughStage::Transform.failed(source))?;
        data.push(logged);
    }
    Ok(EmbeddingResponse {
        object: decoded.object.clone(),
        data,
        model: decoded.model.clone(),
        usage: decoded.usage.clone(),
        hidden_params: IndexMap::new(),
    })
}
