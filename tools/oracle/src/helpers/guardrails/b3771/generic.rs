use std::collections::BTreeMap;
use std::net::TcpListener;
use std::time::Duration;

use betterlite_guardrails::generic_guardrail_api::{
    GenericGuardrailAPI, GenericGuardrailAPIError, extract_user_api_key_metadata,
};
use betterlite_guardrails::generic_guardrail_contracts::{
    GenericGuardrailAPIMetadata, GenericGuardrailAPIRequest, GenericGuardrailAPIResponse,
    GuardrailInputType, StreamingTransformMode, UnreachableFallback,
};
use serde_json::json;

use crate::case::UpstreamAnswer;
use crate::mock::MockProvider;
use crate::routing_mock_scripts::MockScript;

use super::support::{guardrail_error, parse_json_field};

pub(super) fn metadata(input: &serde_json::Value) -> crate::helpers::HelperResult {
    let litellm_values = metadata_field(input, "litellm_metadata")?;
    let metadata_values = metadata_field(input, "metadata")?;
    let merged = extract_user_api_key_metadata(Some(&litellm_values), Some(&metadata_values));
    let serialized = serde_json::to_value(merged).map_err(guardrail_error)?;
    let mut object = serialized.as_object().cloned().unwrap_or_default();
    object.retain(|_, item| !item.is_null());
    Ok(serde_json::Value::Object(object))
}

fn metadata_field(
    input: &serde_json::Value,
    key: &str,
) -> Result<BTreeMap<String, Option<String>>, Box<crate::helpers::HelperError>> {
    let field = input.get(key).cloned().unwrap_or_else(|| json!({}));
    serde_json::from_value(field).map_err(guardrail_error)
}

pub(super) fn call(input: &serde_json::Value) -> crate::helpers::HelperResult {
    let request = input
        .get("generic_call")
        .cloned()
        .unwrap_or_else(|| json!({}));
    run_call(&request)
}

fn run_call(request: &serde_json::Value) -> crate::helpers::HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(guardrail_error)?;
    let settings = settings(request);
    let answer: Option<UpstreamAnswer> = parse_json_field(request, "answer")?;
    let silent = TcpListener::bind(("127.0.0.1", 0)).map_err(guardrail_error)?;
    let base;
    let mock = if let Some(reply) = &answer {
        let script = MockScript::try_from(std::slice::from_ref(reply)).map_err(guardrail_error)?;
        let provider = MockProvider::start(script).map_err(guardrail_error)?;
        base = provider.base_url.clone();
        Some(provider)
    } else {
        base = format!("http://{}", silent.local_addr().map_err(guardrail_error)?);
        None
    };
    let guardrail = GenericGuardrailAPI {
        api_key: None,
        api_base: base,
        settings,
        provider_params: BTreeMap::new(),
        client: reqwest::Client::default(),
        timeout: Duration::from_millis(200),
    };
    let result = runtime.block_on(guardrail.apply_guardrail(request_payload(request)?));
    if let Some(provider) = mock {
        provider.finish().map_err(guardrail_error)?;
    }
    result_payload(result)
}

fn result_payload(
    result: Result<GenericGuardrailAPIResponse, GenericGuardrailAPIError>,
) -> crate::helpers::HelperResult {
    match result {
        Ok(response) => Ok(json!({
            "texts": response.texts,
            "images": response.images,
            "tools": response.tools,
            "structured_messages": response.structured_messages,
            "blocked": false,
        })),
        Err(error) => {
            let is_blocked = matches!(error, GenericGuardrailAPIError::Blocked { .. });
            Err(super::support::error_failure(
                is_blocked,
                "GuardrailRaisedException",
                "Exception",
                error.to_string(),
            ))
        }
    }
}

fn settings(
    request: &serde_json::Value,
) -> betterlite_guardrails::generic_guardrail_contracts::GenericGuardrailAPISettings {
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
    betterlite_guardrails::generic_guardrail_contracts::GenericGuardrailAPISettings {
        unreachable_fallback,
        fail_on_error,
        streaming_end_of_stream_only: false,
        streaming_sampling_rate: 5,
        streaming_transform_mode: StreamingTransformMode::BlockOnly,
    }
}

fn request_payload(
    request: &serde_json::Value,
) -> Result<GenericGuardrailAPIRequest, Box<crate::helpers::HelperError>> {
    let has_inputs = request.get("inputs").is_some();
    let inputs = request.get("inputs").cloned().unwrap_or_else(|| json!({}));
    let texts = parse_json_field::<Vec<String>>(&inputs, "texts")?.or_else(|| {
        Some(if has_inputs {
            Vec::new()
        } else {
            vec!["hello".to_owned()]
        })
    });
    let images = parse_json_field(&inputs, "images")?;
    let tools = parse_json_field(&inputs, "tools")?;
    let structured_messages = parse_json_field(&inputs, "structured_messages")?;
    Ok(GenericGuardrailAPIRequest {
        input_type: GuardrailInputType::Request,
        litellm_call_id: None,
        litellm_trace_id: None,
        structured_messages,
        images,
        tools,
        texts,
        request_data: GenericGuardrailAPIMetadata::default(),
        request_headers: None,
        litellm_version: None,
        provider_params: None,
        tool_calls: None,
        model: None,
    })
}
