use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
// ast-grep-ignore: no-long-names
use betterlite_core::api::file_stream_logging::handler::{
    StreamingParamsCall, resolve_streaming_request_params, should_stream_file_content,
};
use betterlite_core::api::file_stream_logging::logging::{
    CallbackFailure, FailureCall, FileStreamLogger, SuccessCall,
};
use betterlite_core::api::file_stream_logging::source::{EmptySource, StreamSource};
use betterlite_core::api::file_stream_logging::{FileContentStreamingResponse, FileStreamSetup};
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_models::controls::call_params::TrustedCredentials;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use chrono::{DateTime, FixedOffset, Utc};
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    StreamingProjection {
        custom_llm_provider: String,
        file_id: String,
        data: JsonObject,
        should_route: bool,
        original_file_id: Option<String>,
        credentials: Option<JsonObject>,
        #[serde(default)]
        predicates: Vec<Option<String>>,
    },
    HiddenStreamMetadata {
        file_id: String,
        model: Option<String>,
        custom_llm_provider: Option<String>,
        details: Details,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Details {
    model: String,
    api_base: Option<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answers = input.calls.iter().map(answer).collect::<Vec<_>>();
        Ok(Value::Array(answers))
    }
}

fn answer(call: &Call) -> Value {
    match call {
        Call::StreamingProjection {
            custom_llm_provider,
            file_id,
            data,
            should_route,
            original_file_id,
            credentials,
            predicates,
        } => {
            let predicates = predicates
                .iter()
                .map(|provider| should_stream_file_content(provider.as_deref()))
                .collect::<Vec<_>>();
            // ast-grep-ignore: no-long-names
            let resolved = resolve_streaming_request_params(StreamingParamsCall {
                custom_llm_provider,
                file_id,
                data,
                should_route: *should_route,
                original_file_id: original_file_id.as_deref(),
                credentials: credentials.as_ref(),
            });
            match resolved {
                Ok(resolved) => json!({
                    "function": "streaming_projection",
                    "provider": resolved.provider,
                    "file_id": resolved.file_id,
                    "data": resolved.data,
                    "trusted": resolved.trusted_credentials.as_ref().map(snapshot_view),
                    "predicates": predicates,
                }),
                Err(error) => json!({
                    "function": "streaming_projection",
                    "error": error.to_string(),
                    "predicates": predicates,
                }),
            }
        }
        Call::HiddenStreamMetadata {
            file_id,
            model,
            custom_llm_provider,
            details,
        } => hidden_answer(file_id, model, custom_llm_provider, details),
    }
}

fn hidden_answer(
    file_id: &str,
    model: &Option<String>,
    custom_llm_provider: &Option<String>,
    details: &Details,
) -> Value {
    let logger: Arc<dyn FileStreamLogger> = Arc::new(CaptureLogger {
        api_base: details.api_base.clone(),
        payload: payload_fixture(
            file_id,
            &details.model,
            details.api_base.as_deref().unwrap_or(""),
        ),
    });
    let mut stream = FileContentStreamingResponse::new(
        FileStreamSetup {
            source: StreamSource::Sync(Box::new(EmptySource)),
            file_id: file_id.to_owned(),
            model: model.clone(),
            custom_llm_provider: custom_llm_provider.clone(),
            logger: Some(logger),
        },
        Arc::new(WallClock),
    );
    if let Ok(mut chunks) = stream.sync_iter() {
        while chunks.next_chunk().is_some() {}
    }
    json!({
        "function": "hidden_stream_metadata",
        "payload": stream.standard_logging_object().map(payload_view),
    })
}

fn snapshot_view(snapshot: &TrustedCredentials) -> Value {
    let texts = snapshot
        .values
        .iter()
        .map(|(name, secret)| (name.as_str(), secret.expose_secret()))
        .collect::<BTreeMap<_, _>>();
    json!(texts)
}

struct CaptureLogger {
    api_base: Option<String>,
    payload: Option<StandardLoggingPayload>,
}

#[async_trait]
impl FileStreamLogger for CaptureLogger {
    fn start_time(&self) -> Option<DateTime<FixedOffset>> {
        None
    }

    fn details_api_base(&self) -> Option<String> {
        self.api_base.clone()
    }

    fn build_payload(
        &self,
        _response: &Json,
        _start_time: DateTime<FixedOffset>,
        _end_time: DateTime<FixedOffset>,
    ) -> Option<StandardLoggingPayload> {
        self.payload.clone()
    }

    fn success_handler(&self, _call: SuccessCall) -> Result<(), CallbackFailure> {
        Ok(())
    }

    async fn async_success_handler(&self, _call: SuccessCall) -> Result<(), CallbackFailure> {
        Ok(())
    }

    fn success_sync_callbacks(&self, _call: SuccessCall) -> Result<(), CallbackFailure> {
        Ok(())
    }

    fn failure_handler(&self, _call: FailureCall) -> Result<(), CallbackFailure> {
        Ok(())
    }

    async fn async_failure_handler(&self, _call: FailureCall) -> Result<(), CallbackFailure> {
        Ok(())
    }
}

struct WallClock;

impl SuccessClock for WallClock {
    fn now(&self) -> DateTime<FixedOffset> {
        Utc::now().fixed_offset()
    }
}

fn payload_view(payload: &StandardLoggingPayload) -> Value {
    let view = serde_json::to_value(payload).unwrap_or(Value::Null);
    let hidden = &view["hidden_params"];
    json!({
        "id": view["id"],
        "model": view["model"],
        "api_base": view["api_base"],
        "custom_llm_provider": view["custom_llm_provider"],
        "status": view["status"],
        "hidden": {
            "api_base": hidden["api_base"],
            "litellm_model_name": hidden["litellm_model_name"],
            "response_cost": hidden["response_cost"],
        },
        "response": view["response"],
    })
}

fn payload_fixture(id: &str, model: &str, api_base: &str) -> Option<StandardLoggingPayload> {
    let payload = json!({
        "id": id,
        "trace_id": "trace-1",
        "session_id": "session-1",
        "call_type": "afile_content",
        "response_cost": 0.0,
        "status": "success",
        "status_fields": {"llm_api_status": "success", "guardrail_status": "not_run"},
        "total_tokens": 0,
        "prompt_tokens": 0,
        "completion_tokens": 0,
        "startTime": 1767225600.0,
        "endTime": 1767225601.0,
        "completionStartTime": 1767225600.0,
        "response_time": 1.0,
        "model_map_information": {},
        "model": model,
        "api_base": api_base,
        "metadata": {},
        "saved_cache_cost": 0.0,
        "request_tags": [],
        "model_parameters": {},
        "hidden_params": {},
    });
    serde_json::from_value(payload).ok()
}
