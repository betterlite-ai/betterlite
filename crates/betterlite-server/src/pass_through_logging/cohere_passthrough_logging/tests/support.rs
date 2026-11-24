use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::cohere_embedding::usage::CohereEmbeddingUsage;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::pass_through_logging::base_passthrough_logging::tests::support::{
    END, FakePayload, FakeProvider, START, json_object, timestamp,
};
use crate::pass_through_logging::base_passthrough_logging::{
    PassthroughChatInput, PassthroughChatLogger, PassthroughLoggingError, UpstreamChatReply,
};
use crate::pass_through_logging::response_payload::ResponseLoggingKwargs;

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
        PassthroughChatLogger::new(provider.clone(), payload.clone()),
        tools.clone(),
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
    fail_decode: bool,
    fail_cost: bool,
    fail_standard: bool,
}

impl FakeTools {
    pub(crate) fn priced(cost: Decimal) -> Self {
        Self::new(cost, false, false, false)
    }

    pub(crate) fn failing_decode() -> Self {
        Self::new(Decimal::new(25, 2), true, false, false)
    }

    pub(crate) fn failing_cost() -> Self {
        Self::new(Decimal::new(25, 2), false, true, false)
    }

    pub(crate) fn failing_standard() -> Self {
        Self::new(Decimal::new(25, 2), false, false, true)
    }

    fn new(cost: Decimal, fail_decode: bool, fail_cost: bool, fail_standard: bool) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            texts: Mutex::new(Vec::new()),
            models: Mutex::new(Vec::new()),
            standard_seen: Mutex::new(Vec::new()),
            response: embedding_response(),
            cost,
            fail_decode,
            fail_cost,
            fail_standard,
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

    fn record_call(&self, name: &'static str) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name);
    }
}

impl CohereEmbeddingTools for FakeTools {
    fn decode_embedding(
        &self,
        input: EmbeddingDecodeInput<'_>,
        _call_log: &mut LoggingState,
    ) -> Result<EmbeddingResponse, PassthroughLoggingError> {
        self.record_call("decode");
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(input.model.to_owned());
        let texts = input
            .input
            .iter()
            .map(|value| match value {
                Json::Text(text) => text.clone(),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    String::new()
                }
            })
            .collect();
        self.texts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(texts);
        if self.fail_decode {
            return Err(PassthroughLoggingError::transform(io::Error::other(
                "the fake decode failed",
            )));
        }
        Ok(self.response.clone())
    }

    fn embedding_cost(
        &self,
        _response: &EmbeddingResponse,
        model: &str,
    ) -> Result<Decimal, PassthroughLoggingError> {
        self.record_call("cost");
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model.to_owned());
        if self.fail_cost {
            return Err(PassthroughLoggingError::transform(io::Error::other(
                "the fake cost failed",
            )));
        }
        Ok(self.cost)
    }

    fn standard_logging_object(
        &self,
        _input: StandardObjectInput<'_>,
        call_log: &mut LoggingState,
    ) -> Result<(), PassthroughLoggingError> {
        self.record_call("standard");
        let carries = ["model", "custom_llm_provider", "response_cost"]
            .iter()
            .all(|key| call_log.model_call_details.contains_key(*key));
        self.standard_seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(carries);
        if self.fail_standard {
            return Err(PassthroughLoggingError::payload(io::Error::other(
                "the fake standard object failed",
            )));
        }
        Ok(())
    }
}

// The keys write_call_log fills, read back from the call log.
pub(crate) fn call_log_detail(call_log: &LoggingState, key: &str) -> Option<Json> {
    call_log.model_call_details.get(key).cloned()
}
