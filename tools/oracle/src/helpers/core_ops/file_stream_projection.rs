use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_core::api::file_stream_logging::handler::{
    StreamingParamsCall, resolve_streaming_params, should_stream_file_content,
};
use betterlite_core::api::file_stream_logging::source::{EmptySource, StreamSource};
use betterlite_core::api::file_stream_logging::{FileContentStreamingResponse, FileStreamSetup};
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_models::controls::call_params::TrustedCredentials;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::{Value, json};

use fixtures::support::{CaptureLogger, WallClock, payload_fixture};

mod fixtures;

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
        } => projection_answer(
            &StreamingParamsCall {
                custom_llm_provider,
                file_id,
                data,
                should_route: *should_route,
                original_file_id: original_file_id.as_deref(),
                credentials: credentials.as_ref(),
            },
            predicates,
        ),
        Call::HiddenStreamMetadata {
            file_id,
            model,
            custom_llm_provider,
            details,
        } => hidden_answer(
            file_id,
            model.as_deref(),
            custom_llm_provider.as_deref(),
            details,
        ),
    }
}

fn projection_answer(params: &StreamingParamsCall<'_>, predicates: &[Option<String>]) -> Value {
    let accepted = predicates
        .iter()
        .map(|provider| should_stream_file_content(provider.as_deref()))
        .collect::<Vec<_>>();
    match resolve_streaming_params(params) {
        Ok(resolved) => json!({
            "function": "streaming_projection",
            "provider": resolved.provider,
            "file_id": resolved.file_id,
            "data": resolved.data,
            "trusted": resolved.trusted_credentials.as_ref().map(snapshot_view),
            "predicates": accepted,
        }),
        Err(error) => json!({
            "function": "streaming_projection",
            "error": error.to_string(),
            "predicates": accepted,
        }),
    }
}

fn hidden_answer(
    file_id: &str,
    model: Option<&str>,
    custom_llm_provider: Option<&str>,
    details: &Details,
) -> Value {
    let logger = Arc::new(CaptureLogger {
        api_base: details.api_base.clone(),
        payload: payload_fixture(
            file_id,
            &details.model,
            details.api_base.as_deref().unwrap_or(""),
        ),
        recorded: Mutex::new(None),
    });
    let mut stream = FileContentStreamingResponse::new(
        FileStreamSetup {
            source: StreamSource::Sync(Box::new(EmptySource)),
            file_id: file_id.to_owned(),
            model: model.map(str::to_owned),
            custom_llm_provider: custom_llm_provider.map(str::to_owned),
            logger: Some(Arc::<CaptureLogger>::clone(&logger)),
        },
        Arc::new(WallClock),
    );
    if let Ok(mut chunks) = stream.sync_iter() {
        while chunks.next_chunk().is_some() {}
    }
    let payload = logger
        .recorded
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    json!({
        "function": "hidden_stream_metadata",
        "payload": payload.as_ref().map(payload_view),
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

fn payload_view(payload: &StandardLoggingPayload) -> Value {
    let view = serde_json::to_value(payload).unwrap_or(Value::Null);
    let hidden = view.get("hidden_params").cloned().unwrap_or(Value::Null);
    json!({
        "id": view.get("id").cloned(),
        "model": view.get("model").cloned(),
        "api_base": view.get("api_base").cloned(),
        "custom_llm_provider": view.get("custom_llm_provider").cloned(),
        "status": view.get("status").cloned(),
        "hidden": {
            "api_base": hidden.get("api_base").cloned(),
            "litellm_model_name": hidden.get("litellm_model_name").cloned(),
            "response_cost": hidden.get("response_cost").cloned(),
        },
        "response": view.get("response").cloned(),
    })
}
