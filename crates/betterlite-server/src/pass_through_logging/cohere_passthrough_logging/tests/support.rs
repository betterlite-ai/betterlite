use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_core::openai_pricing::cost_error::CostError;
use betterlite_core::openai_pricing::{CompletionCost, CompletionCostRequest, CostedResponse};
use betterlite_core::routes::CallType;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::cohere_embedding::usage::CohereEmbeddingUsage;
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::assets;
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};
use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::pass_through_logging::base_passthrough_logging::tests::support::{
    END, FakePayload, FakeProvider, RecordingBuilder, START, json_object, timestamp,
};
use crate::pass_through_logging::base_passthrough_logging::{
    PassthroughChatInput, PassthroughChatLogger, PassthroughLoggingError, PassthroughStage,
    UpstreamChatReply,
};
use crate::pass_through_logging::response_payload::ResponseLoggingKwargs;

use super::super::embedding::CohereEmbeddingLogging;
use super::super::{
    CohereEmbeddingTools, CoherePassthroughLogger, EmbeddingDecodeInput, EmbeddingResponse,
    StandardObjectInput,
};

// The handler's input with a fixed upstream reply and the workspace's test timestamps.
pub(crate) fn input<'input>(
    request_body: &'input JsonObject,
    response_body: &'input JsonObject,
    call_log: &'input mut LoggingState,
    url_route: &'input str,
    kwargs: ResponseLoggingKwargs,
) -> PassthroughChatInput<'input> {
    PassthroughChatInput {
        upstream: UpstreamChatReply {
            status_code: 200,
            body: b"{}",
        },
        response_body,
        call_log,
        url_route,
        start_time: timestamp(START),
        end_time: timestamp(END),
        request_body,
        kwargs,
    }
}

pub(crate) fn cohere_logger(
    provider: &Arc<FakeProvider>,
    payload: &Arc<FakePayload>,
    tools: &Arc<FakeTools>,
) -> CoherePassthroughLogger {
    CoherePassthroughLogger::new(
        PassthroughChatLogger::new(
            Arc::<FakeProvider>::clone(provider),
            Arc::<FakePayload>::clone(payload),
        ),
        Arc::<FakeTools>::clone(tools),
    )
}

pub(crate) fn embedding_response() -> EmbeddingResponse {
    EmbeddingResponse {
        object: "list".to_owned(),
        data: vec![json_object(
            r#"{"object": "embedding", "index": 0, "embedding": [0.1, 0.2]}"#,
        )],
        model: "embed-model".to_owned(),
        usage: CohereEmbeddingUsage {
            prompt_tokens: 4,
            completion_tokens: 0,
            total_tokens: 4,
            prompt_tokens_details: None,
        },
        hidden_params: IndexMap::new(),
    }
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("a JSON value")
}

// The three collaborator calls of the embed branch, recording what each was asked and when.
pub(crate) struct FakeTools {
    calls: Mutex<Vec<&'static str>>,
    texts: Mutex<Vec<Vec<String>>>,
    models: Mutex<Vec<String>>,
    standard_seen: Mutex<Vec<bool>>,
    response: EmbeddingResponse,
    cost: Decimal,
    failing_step: Option<&'static str>,
}

impl FakeTools {
    pub(crate) fn priced(cost: Decimal) -> Self {
        Self::new(cost, None)
    }

    pub(crate) fn failing_decode() -> Self {
        Self::new(Decimal::new(25, 2), Some("decode"))
    }

    pub(crate) fn failing_cost() -> Self {
        Self::new(Decimal::new(25, 2), Some("cost"))
    }

    pub(crate) fn failing_standard() -> Self {
        Self::new(Decimal::new(25, 2), Some("standard"))
    }

    fn new(cost: Decimal, failing_step: Option<&'static str>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            texts: Mutex::new(Vec::new()),
            models: Mutex::new(Vec::new()),
            standard_seen: Mutex::new(Vec::new()),
            response: embedding_response(),
            cost,
            failing_step,
        }
    }

    pub(crate) fn calls(&self) -> Vec<&'static str> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn texts(&self) -> Vec<Vec<String>> {
        self.texts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn models(&self) -> Vec<String> {
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn standard_seen(&self) -> Vec<bool> {
        self.standard_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    // Records the call and fails it when it is the step this fake was told to fail.
    fn step(&self, name: &'static str) -> Result<(), PassthroughLoggingError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name);
        if self.failing_step == Some(name) {
            return Err(PassthroughStage::Transform
                .failed(io::Error::other(format!("the fake {name} step failed"))));
        }
        Ok(())
    }

    fn record_model(&self, model: &str) {
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model.to_owned());
    }
}

// The text a fake decode records for one input value; a value that is not text records as empty.
fn recorded_text(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            String::new()
        }
    }
}

impl CohereEmbeddingTools for FakeTools {
    fn decode_embedding(
        &self,
        input: EmbeddingDecodeInput<'_>,
        _call_log: &mut LoggingState,
    ) -> Result<EmbeddingResponse, PassthroughLoggingError> {
        self.record_model(input.model);
        self.texts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(input.input.iter().map(recorded_text).collect());
        self.step("decode")?;
        Ok(self.response.clone())
    }

    fn embedding_cost(
        &self,
        _response: &EmbeddingResponse,
        model: &str,
    ) -> Result<Decimal, PassthroughLoggingError> {
        self.record_model(model);
        self.step("cost")?;
        Ok(self.cost)
    }

    fn standard_logging_object(
        &self,
        _input: StandardObjectInput<'_>,
        call_log: &mut LoggingState,
    ) -> Result<(), PassthroughLoggingError> {
        let carries = ["model", "custom_llm_provider", "response_cost"]
            .iter()
            .all(|key| call_log.model_call_details.contains_key(*key));
        self.standard_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(carries);
        self.step("standard")
    }
}

// One priced request as B068's completion_cost receives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PricedCall {
    pub(crate) model: String,
    pub(crate) provider: String,
    pub(crate) call_type: CallType,
    pub(crate) response_model: Option<String>,
}

// B068's pricing at a fixed price, recording every request it priced.
pub(crate) struct RecordingPricing {
    price: Decimal,
    calls: Mutex<Vec<PricedCall>>,
}

impl RecordingPricing {
    pub(crate) fn new(price: Decimal) -> Self {
        Self {
            price,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<PricedCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CompletionCost for RecordingPricing {
    fn completion_cost(&self, request: &CompletionCostRequest<'_>) -> Result<Decimal, CostError> {
        let response_model = match request.response {
            CostedResponse::Embedding(embedding) => Some(embedding.model.clone()),
            CostedResponse::Responses(_) => None,
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(PricedCall {
                model: request.model.to_owned(),
                provider: request.custom_llm_provider.to_owned(),
                call_type: request.call_type,
                response_model,
            });
        Ok(self.price)
    }
}

// The production embedding tools over the fakes and the bundled cl100k tokenizer.
pub(crate) fn production_tools(
    pricing: &Arc<RecordingPricing>,
    builder: &Arc<RecordingBuilder>,
) -> CohereEmbeddingLogging {
    let vocabulary = assets::bundled(Encoding::Cl100kBase).expect("the bundled cl100k vocabulary");
    let engine = TiktokenEngine::from_bpe_file(Encoding::Cl100kBase, &vocabulary)
        .expect("the cl100k engine builds");
    CohereEmbeddingLogging::new(
        Arc::<RecordingPricing>::clone(pricing),
        Arc::<RecordingBuilder>::clone(builder),
        Engine::Tiktoken(Arc::new(engine)),
    )
}

// The keys write_call_log fills, read back from the call log.
pub(crate) fn call_log_detail(call_log: &LoggingState, key: &str) -> Option<Json> {
    call_log.model_call_details.get(key).cloned()
}
