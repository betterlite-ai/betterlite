use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_guardrails::grayswan::error::GraySwanGuardrailAPIError;
use betterlite_guardrails::grayswan::wire::{GraySwanDynamicParams, GraySwanInput, GraySwanResponse};
use betterlite_guardrails::grayswan::{GraySwanGuardrail, GraySwanSettings, OnFlaggedAction};
use betterlite_guardrails::generic_guardrail_contracts::{
    GuardrailExtraValue, GuardrailInputType, GuardrailMessage, GuardrailToolParam,
};
use secrecy::SecretString;
use serde_json::json;

use crate::case::UpstreamAnswer;
use crate::mock::MockProvider;

use super::support::{
    current_thread_runtime, disconnection_script, error_failure, field_or_empty_object,
    guardrail_error, mock_script_for, parse_json_field,
};

pub(super) fn verdict(input: &serde_json::Value) -> crate::helpers::HelperResult {
    let response = GraySwanResponse {
        violation: input
            .get("score")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0_f64),
        violated_rule_descriptions: Vec::new(),
        mutation: None,
        ipi: None,
    };
    let value = guardrail(input)?
        .process_response(&response, input_type(input), Vec::new())
        .map_err(|error| failure(&error))?;
    Ok(json!({
        "allowed": true,
        "flagged": value.flagged,
        "replaced": !value.messages.is_empty(),
    }))
}

fn input_type(input: &serde_json::Value) -> GuardrailInputType {
    let is_output = input
        .get("is_output")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if is_output {
        GuardrailInputType::Response
    } else {
        GuardrailInputType::Request
    }
}

fn guardrail(input: &serde_json::Value) -> Result<GraySwanGuardrail, Box<crate::helpers::HelperError>> {
    let settings = parse_settings(input, OnFlaggedAction::Block)?;
    Ok(make_guardrail(
        "http://oracle.invalid".to_owned(),
        settings,
        betterlite_guardrails::grayswan::DEFAULT_TIMEOUT,
    ))
}

fn make_guardrail(api_base: String, settings: GraySwanSettings, timeout: Duration) -> GraySwanGuardrail {
    GraySwanGuardrail {
        api_key: SecretString::from("oracle-key"),
        api_base,
        settings,
        client: reqwest::Client::default(),
        timeout,
    }
}

fn parse_settings(
    value: &serde_json::Value,
    default_action: OnFlaggedAction,
) -> Result<GraySwanSettings, Box<crate::helpers::HelperError>> {
    let action = match value.get("action").and_then(serde_json::Value::as_str) {
        Some("BLOCK") => OnFlaggedAction::Block,
        Some("MONITOR") => OnFlaggedAction::Monitor,
        Some("PASSTHROUGH") => OnFlaggedAction::Passthrough,
        _ => default_action,
    };
    let violation_threshold = value
        .get("violation_threshold")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.5_f64);
    let fail_open = value
        .get("fail_open")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    let policy_id = value
        .get("policy_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    Ok(GraySwanSettings {
        on_flagged_action: action,
        violation_threshold,
        reasoning_mode: parse_json_field(value, "reasoning_mode")?,
        categories: parse_json_field(value, "categories")?,
        policy_id,
        fail_open,
    })
}

struct Request {
    input_type: GuardrailInputType,
    settings: GraySwanSettings,
    dynamic_params: GraySwanDynamicParams,
    tools: Vec<GuardrailToolParam>,
    messages: Vec<GuardrailMessage>,
    texts: Vec<String>,
    tool_calls: Vec<GuardrailExtraValue>,
    context: Vec<GuardrailMessage>,
    answer: Option<UpstreamAnswer>,
    fail: String,
}

// grayswan_payload nests settings under "config"; grayswan_call puts them at the top level.
fn parse_request(request: &serde_json::Value) -> Result<Request, Box<crate::helpers::HelperError>> {
    let settings_source = request
        .get("config")
        .cloned()
        .unwrap_or_else(|| request.clone());
    let settings = parse_settings(&settings_source, OnFlaggedAction::Monitor)?;
    let dynamic = field_or_empty_object(request, "dynamic");
    let dynamic_params: GraySwanDynamicParams =
        serde_json::from_value(dynamic).map_err(guardrail_error)?;
    let tools = parse_json_field(request, "tools")?.unwrap_or_default();
    let fallback_message = || {
        Ok(vec![GuardrailMessage {
            role: "user".to_owned(),
            extra: BTreeMap::from([(
                "content".to_owned(),
                GuardrailExtraValue::Text("hello".to_owned()),
            )]),
        }])
    };
    let messages = request
        .get("messages")
        .map_or_else(fallback_message, |value| {
            serde_json::from_value(value.clone()).map_err(guardrail_error)
        })?;
    let (input_type, fail) = request_flags(request);
    let texts = parse_json_field(request, "texts")?.unwrap_or_default();
    let tool_calls = parse_json_field(request, "tool_calls")?.unwrap_or_default();
    let context = parse_json_field(request, "context")?.unwrap_or_default();
    let answer = parse_json_field(request, "answer")?;
    Ok(Request {
        input_type,
        settings,
        dynamic_params,
        tools,
        messages,
        texts,
        tool_calls,
        context,
        answer,
        fail,
    })
}

fn request_flags(request: &serde_json::Value) -> (GuardrailInputType, String) {
    let input_type = match request
        .get("input_type")
        .and_then(serde_json::Value::as_str)
    {
        Some("response") => GuardrailInputType::Response,
        _ => GuardrailInputType::Request,
    };
    let fail = request
        .get("fail")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("timeout")
        .to_owned();
    (input_type, fail)
}

pub(super) fn payload(input: &serde_json::Value) -> crate::helpers::HelperResult {
    let request = field_or_empty_object(input, "grayswan_payload");
    let parsed = parse_request(&request)?;
    run_payload(parsed)
}

fn run_payload(parsed: Request) -> crate::helpers::HelperResult {
    let guardrail = make_guardrail(
        "http://oracle.invalid".to_owned(),
        parsed.settings,
        betterlite_guardrails::grayswan::DEFAULT_TIMEOUT,
    );
    let prepared = build_input(
        Vec::new(),
        Vec::new(),
        parsed.tools,
        parsed.dynamic_params,
        Vec::new(),
    );
    let payload = guardrail.prepare_payload(&prepared, parsed.messages);
    serde_json::to_value(payload).map_err(guardrail_error)
}

pub(super) fn call(input: &serde_json::Value) -> crate::helpers::HelperResult {
    let request = field_or_empty_object(input, "grayswan_call");
    let call = parse_request(&request)?;
    run_call(call)
}

fn run_call(call: Request) -> crate::helpers::HelperResult {
    let script = match &call.answer {
        Some(reply) => mock_script_for(reply)?,
        None => disconnection_script(&call.fail),
    };
    let mock = MockProvider::start(script).map_err(guardrail_error)?;
    let guardrail = make_guardrail(mock.base_url.clone(), call.settings, Duration::from_millis(200));
    let prepared = build_input(
        call.texts,
        call.tool_calls,
        Vec::new(),
        GraySwanDynamicParams::default(),
        call.context,
    );
    let runtime = current_thread_runtime()?;
    let result = runtime.block_on(guardrail.apply_guardrail(prepared, call.input_type));
    let calls = mock.finish().map_err(guardrail_error)?;
    let payload = calls
        .into_iter()
        .next()
        .map(|received| received.request.body);
    result
        .map(|_outcome| payload.unwrap_or(serde_json::Value::Null))
        .map_err(|error| failure(&error))
}

fn failure(error: &GraySwanGuardrailAPIError) -> Box<crate::helpers::HelperError> {
    let is_blocked = matches!(error, GraySwanGuardrailAPIError::Blocked { .. });
    error_failure(
        is_blocked,
        "HTTPException",
        "GraySwanGuardrailAPIError",
        error.to_string(),
    )
}

const fn build_input(
    texts: Vec<String>,
    tool_calls: Vec<GuardrailExtraValue>,
    tools: Vec<GuardrailToolParam>,
    metadata: GraySwanDynamicParams,
    request_history: Vec<GuardrailMessage>,
) -> GraySwanInput {
    GraySwanInput {
        texts,
        tool_calls,
        tools,
        metadata,
        request_history,
        litellm_metadata: BTreeMap::new(),
    }
}
