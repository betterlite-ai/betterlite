use std::net::TcpListener;
use std::time::Duration;

use betterlite_guardrails::generic_guardrail_api::{
    GenericGuardrailAPI, GenericGuardrailAPIError, GenericGuardrailAPITransport,
    extract_user_api_key_metadata,
};
use betterlite_guardrails::generic_guardrail_contracts::{
    GenericGuardrailAPIMetadata, GenericGuardrailAPIRequest, GenericGuardrailAPISettings,
    GuardrailInputType, StreamingTransformMode, UnreachableFallback,
};
use betterlite_guardrails::grayswan::error::GraySwanGuardrailAPIError;
use betterlite_guardrails::grayswan::wire::GraySwanResponse;
use betterlite_guardrails::grayswan::{GraySwanGuardrail, GraySwanSettings, OnFlaggedAction};
use secrecy::SecretString;
use serde_json::json;
use std::collections::BTreeMap;

use crate::case::UpstreamAnswer;
use crate::mock::MockProvider;
use crate::routing_mock_scripts::MockScript;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        if input.get("generic_call").is_some() {
            generic_call(input)
        } else if input.get("metadata").is_some() || input.get("litellm_metadata").is_some() {
            metadata(input)
        } else if input.get("score").is_some() || input.get("action").is_some() {
            grayswan_verdict(input)
        } else {
            Self::not_built("b3771")
        }
    }
}

fn metadata(input: &serde_json::Value) -> crate::helpers::HelperResult {
    let litellm_json = input
        .get("litellm_metadata")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let metadata_json = input.get("metadata").cloned().unwrap_or_else(|| json!({}));
    let litellm_values = serde_json::from_value::<BTreeMap<String, Option<String>>>(litellm_json)
        .map_err(|error| failure(error.to_string()))?;
    let metadata_values = serde_json::from_value::<BTreeMap<String, Option<String>>>(metadata_json)
        .map_err(|error| failure(error.to_string()))?;
    let merged = extract_user_api_key_metadata(Some(&litellm_values), Some(&metadata_values));
    let serialized = serde_json::to_value(merged).map_err(|error| failure(error.to_string()))?;
    let object = serialized.as_object().cloned().unwrap_or_default();
    Ok(serde_json::Value::Object(
        object
            .into_iter()
            .filter(|(_, item)| !item.is_null())
            .collect(),
    ))
}

fn grayswan_verdict(input: &serde_json::Value) -> crate::helpers::HelperResult {
    let guardrail = grayswan_guardrail(input);
    let response = GraySwanResponse {
        violation: input
            .get("score")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0_f64),
        violated_rule_descriptions: Vec::new(),
        mutation: None,
        ipi: None,
    };
    let is_output = input
        .get("is_output")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let input_type = if is_output {
        GuardrailInputType::Response
    } else {
        GuardrailInputType::Request
    };
    let value = guardrail
        .process_response(&response, input_type, Vec::new())
        .map_err(|error| grayswan_failure(&error))?;
    serde_json::to_value(json!({"allowed": true, "flagged": value.flagged}))
        .map_err(|error| failure(error.to_string()))
}

fn grayswan_guardrail(input: &serde_json::Value) -> GraySwanGuardrail {
    let action = match input.get("action").and_then(serde_json::Value::as_str) {
        Some("PASSTHROUGH") => OnFlaggedAction::Passthrough,
        Some("MONITOR") => OnFlaggedAction::Monitor,
        _ => OnFlaggedAction::Block,
    };
    GraySwanGuardrail::with_client(
        SecretString::from("oracle-key"),
        "http://oracle.invalid".to_owned(),
        GraySwanSettings {
            on_flagged_action: action,
            violation_threshold: 0.5,
            reasoning_mode: None,
            categories: None,
            policy_id: None,
            fail_open: true,
        },
        reqwest::Client::default(),
        betterlite_guardrails::grayswan::DEFAULT_TIMEOUT,
    )
}

fn generic_call(input: &serde_json::Value) -> crate::helpers::HelperResult {
    let request = input
        .get("generic_call")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let settings = generic_settings(&request);
    let answer = request
        .get("answer")
        .cloned()
        .map(serde_json::from_value::<UpstreamAnswer>)
        .transpose()
        .map_err(|error| failure(error.to_string()))?;

    let io = |error: std::io::Error| failure(error.to_string());
    let mock_error = |error: crate::OracleError| failure(error.to_string());
    let silent = TcpListener::bind(("127.0.0.1", 0)).map_err(io)?;
    let start = |reply: &UpstreamAnswer| {
        MockScript::try_from(std::slice::from_ref(reply)).and_then(MockProvider::start)
    };
    let mock = answer.as_ref().map(start).transpose().map_err(mock_error)?;
    let base = match &mock {
        Some(provider) => provider.base_url.clone(),
        None => format!("http://{}", silent.local_addr().map_err(io)?),
    };

    let guardrail = generic_guardrail(&base, settings);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(io)?;
    let result = runtime.block_on(guardrail.apply_guardrail(generic_request()));
    mock.map(MockProvider::finish)
        .transpose()
        .map_err(mock_error)?;
    match result {
        Ok(response) => serde_json::to_value(json!({"texts": response.texts, "blocked": false}))
            .map_err(|error| failure(error.to_string())),
        Err(error) => Err(generic_failure(&error)),
    }
}

fn generic_settings(request: &serde_json::Value) -> GenericGuardrailAPISettings {
    let unreachable_fallback = match request
        .get("unreachable_fallback")
        .and_then(serde_json::Value::as_str)
    {
        Some("fail_open") => UnreachableFallback::FailOpen,
        _ => UnreachableFallback::FailClosed,
    };
    let fail_on_error = request
        .get("fail_on_error")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    GenericGuardrailAPISettings {
        unreachable_fallback,
        fail_on_error,
        streaming_end_of_stream_only: false,
        streaming_sampling_rate: 5,
        streaming_transform_mode: StreamingTransformMode::BlockOnly,
    }
}

fn generic_guardrail(base: &str, settings: GenericGuardrailAPISettings) -> GenericGuardrailAPI {
    GenericGuardrailAPI::with_client(
        None,
        base,
        settings,
        None,
        GenericGuardrailAPITransport {
            client: reqwest::Client::default(),
            timeout: Duration::from_millis(200),
        },
    )
}

fn generic_request() -> GenericGuardrailAPIRequest {
    GenericGuardrailAPIRequest {
        input_type: GuardrailInputType::Request,
        litellm_call_id: None,
        litellm_trace_id: None,
        structured_messages: None,
        images: None,
        tools: None,
        texts: Some(vec!["hello".to_owned()]),
        request_data: GenericGuardrailAPIMetadata::default(),
        request_headers: None,
        litellm_version: None,
        provider_params: None,
        tool_calls: None,
        model: None,
    }
}

fn generic_failure(error: &GenericGuardrailAPIError) -> Box<crate::helpers::HelperError> {
    let class = match error {
        GenericGuardrailAPIError::Blocked { .. } => "GuardrailRaisedException",
        GenericGuardrailAPIError::Timeout { .. }
        | GenericGuardrailAPIError::Request { .. }
        | GenericGuardrailAPIError::InvalidHeader { .. }
        | GenericGuardrailAPIError::HttpStatus { .. }
        | GenericGuardrailAPIError::ResponseTooLarge { .. }
        | GenericGuardrailAPIError::ResponseJson { .. } => "Exception",
    };
    failure_with_class(class, error.to_string())
}

fn grayswan_failure(error: &GraySwanGuardrailAPIError) -> Box<crate::helpers::HelperError> {
    failure_with_class("HTTPException", error.to_string())
}

fn failure(message: String) -> Box<crate::helpers::HelperError> {
    failure_with_class("GuardrailError", message)
}

fn failure_with_class(class_name: &str, message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: class_name.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
