use std::net::TcpListener;
use std::time::Duration;

use betterlite_guardrails::generic_guardrail_api::{
    GenericGuardrailAPI, GenericGuardrailAPIError, GenericGuardrailAPITransport,
    extract_user_api_key_metadata,
};
use betterlite_guardrails::generic_guardrail_contracts::{
    GenericGuardrailAPIMetadata, GenericGuardrailAPIRequest, GenericGuardrailAPISettings,
    GuardrailExtraValue, GuardrailInputType, GuardrailMessage, GuardrailToolParam,
    StreamingTransformMode, UnreachableFallback,
};
use betterlite_guardrails::grayswan::error::GraySwanGuardrailAPIError;
use betterlite_guardrails::grayswan::wire::{
    GraySwanDynamicParams, GraySwanInput, GraySwanResponse,
};
use betterlite_guardrails::grayswan::{GraySwanGuardrail, GraySwanSettings, OnFlaggedAction};
use secrecy::SecretString;
use serde_json::json;
use std::collections::BTreeMap;

use crate::case::UpstreamAnswer;
use crate::mock::MockProvider;
use crate::routing_mock_scripts::{MockScript, Outcome, ScriptedReply, Wait};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        if input.get("generic_call").is_some() {
            generic_call(input)
        } else if input.get("grayswan_call").is_some() {
            grayswan_call(input)
        } else if input.get("grayswan_payload").is_some() {
            grayswan_payload(input)
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

fn grayswan_verdict(input: &serde_json::Value) -> crate::helpers::HelperResult {
    let response = GraySwanResponse {
        violation: input
            .get("score")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0_f64),
        violated_rule_descriptions: Vec::new(),
        mutation: None,
        ipi: None,
    };
    let value = grayswan_guardrail(input)?
        .process_response(&response, grayswan_input_type(input), Vec::new())
        .map_err(|error| grayswan_failure(&error))?;
    Ok(json!({
        "allowed": true,
        "flagged": value.flagged,
        "replaced": !value.messages.is_empty(),
    }))
}

fn grayswan_input_type(input: &serde_json::Value) -> GuardrailInputType {
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

fn grayswan_guardrail(
    input: &serde_json::Value,
) -> Result<GraySwanGuardrail, Box<crate::helpers::HelperError>> {
    let settings = parse_grayswan_settings(input, OnFlaggedAction::Block)?;
    Ok(make_grayswan_guardrail(
        "http://oracle.invalid".to_owned(),
        settings,
        betterlite_guardrails::grayswan::DEFAULT_TIMEOUT,
    ))
}

fn make_grayswan_guardrail(
    api_base: String,
    settings: GraySwanSettings,
    timeout: Duration,
) -> GraySwanGuardrail {
    GraySwanGuardrail {
        api_key: SecretString::from("oracle-key"),
        api_base,
        settings,
        client: reqwest::Client::default(),
        timeout,
    }
}

fn parse_grayswan_settings(
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

struct GraySwanRequest {
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

fn parse_json_field<T>(
    request: &serde_json::Value,
    name: &str,
) -> Result<Option<T>, Box<crate::helpers::HelperError>>
where
    T: serde::de::DeserializeOwned,
{
    let Some(value) = request.get(name).cloned() else {
        return Ok(None);
    };
    let parsed = serde_json::from_value(value).map_err(guardrail_error)?;
    Ok(Some(parsed))
}

// grayswan_payload nests settings under "config"; grayswan_call puts them at the top level.
fn parse_grayswan_request(
    request: &serde_json::Value,
) -> Result<GraySwanRequest, Box<crate::helpers::HelperError>> {
    let settings_source = request
        .get("config")
        .cloned()
        .unwrap_or_else(|| request.clone());
    let settings = parse_grayswan_settings(&settings_source, OnFlaggedAction::Monitor)?;
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
    let (input_type, fail) = grayswan_request_flags(request);
    let texts = parse_json_field(request, "texts")?.unwrap_or_default();
    let tool_calls = parse_json_field(request, "tool_calls")?.unwrap_or_default();
    let context = parse_json_field(request, "context")?.unwrap_or_default();
    let answer = parse_json_field(request, "answer")?;
    Ok(GraySwanRequest {
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

fn grayswan_request_flags(request: &serde_json::Value) -> (GuardrailInputType, String) {
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

fn grayswan_payload(input: &serde_json::Value) -> crate::helpers::HelperResult {
    let request = field_or_empty_object(input, "grayswan_payload");
    let parsed = parse_grayswan_request(&request)?;
    run_grayswan_payload(parsed)
}

fn run_grayswan_payload(parsed: GraySwanRequest) -> crate::helpers::HelperResult {
    let guardrail = make_grayswan_guardrail(
        "http://oracle.invalid".to_owned(),
        parsed.settings,
        betterlite_guardrails::grayswan::DEFAULT_TIMEOUT,
    );
    let prepared = build_grayswan_input(
        Vec::new(),
        Vec::new(),
        parsed.tools,
        parsed.dynamic_params,
        Vec::new(),
    );
    let payload = guardrail.prepare_payload(&prepared, parsed.messages);
    serde_json::to_value(payload).map_err(guardrail_error)
}

fn grayswan_call(input: &serde_json::Value) -> crate::helpers::HelperResult {
    let request = field_or_empty_object(input, "grayswan_call");
    let call = parse_grayswan_request(&request)?;
    run_grayswan_call(call)
}

fn run_grayswan_call(call: GraySwanRequest) -> crate::helpers::HelperResult {
    let script = match &call.answer {
        Some(reply) => mock_script_for(reply)?,
        None => disconnection_script(&call.fail),
    };
    let mock = MockProvider::start(script).map_err(guardrail_error)?;
    let guardrail = make_grayswan_guardrail(
        mock.base_url.clone(),
        call.settings,
        Duration::from_millis(200),
    );
    let prepared = build_grayswan_input(
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
        .map_err(|error| grayswan_failure(&error))
}

fn mock_script_for(reply: &UpstreamAnswer) -> Result<MockScript, Box<crate::helpers::HelperError>> {
    MockScript::try_from(std::slice::from_ref(reply)).map_err(guardrail_error)
}

// "closed" drops the connection immediately; "timeout" delays past the client's timeout.
fn disconnection_script(fail: &str) -> MockScript {
    let wait = if fail == "closed" {
        Wait::None
    } else {
        Wait::Delay { millis: 500 }
    };
    MockScript {
        deadline: Duration::from_secs(5),
        replies: vec![ScriptedReply {
            wait,
            then: Outcome::Close,
        }],
        barriers: BTreeMap::new(),
    }
}

fn generic_call(input: &serde_json::Value) -> crate::helpers::HelperResult {
    let request = field_or_empty_object(input, "generic_call");
    run_generic_call(&request)
}

fn run_generic_call(request: &serde_json::Value) -> crate::helpers::HelperResult {
    let runtime = current_thread_runtime()?;
    let settings = generic_settings(request);
    let answer: Option<UpstreamAnswer> = parse_json_field(request, "answer")?;
    let silent = TcpListener::bind(("127.0.0.1", 0)).map_err(guardrail_error)?;
    let base;
    let mock = if let Some(reply) = &answer {
        let provider = MockProvider::start(mock_script_for(reply)?).map_err(guardrail_error)?;
        base = provider.base_url.clone();
        Some(provider)
    } else {
        base = format!("http://{}", silent.local_addr().map_err(guardrail_error)?);
        None
    };
    let guardrail = generic_call_guardrail(&base, settings);
    let result = runtime.block_on(guardrail.apply_guardrail(generic_request(request)?));
    if let Some(provider) = mock {
        provider.finish().map_err(guardrail_error)?;
    }
    generic_result(result)
}

fn generic_call_guardrail(
    base: &str,
    settings: GenericGuardrailAPISettings,
) -> GenericGuardrailAPI {
    let transport = GenericGuardrailAPITransport {
        client: reqwest::Client::default(),
        timeout: Duration::from_millis(200),
    };
    GenericGuardrailAPI::with_client(None, base, settings, None, transport)
}

fn generic_result(
    result: Result<
        betterlite_guardrails::generic_guardrail_contracts::GenericGuardrailAPIResponse,
        GenericGuardrailAPIError,
    >,
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
            Err(error_failure(
                is_blocked,
                "GuardrailRaisedException",
                "Exception",
                error.to_string(),
            ))
        }
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

fn generic_request(
    request: &serde_json::Value,
) -> Result<GenericGuardrailAPIRequest, Box<crate::helpers::HelperError>> {
    let has_inputs = request.get("inputs").is_some();
    let inputs = field_or_empty_object(request, "inputs");
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

fn error_failure(
    is_blocked: bool,
    blocked_class: &str,
    other_class: &str,
    message: String,
) -> Box<crate::helpers::HelperError> {
    let class = if is_blocked {
        blocked_class
    } else {
        other_class
    };
    failure_with_class(class, message)
}

fn grayswan_failure(error: &GraySwanGuardrailAPIError) -> Box<crate::helpers::HelperError> {
    let is_blocked = matches!(error, GraySwanGuardrailAPIError::Blocked { .. });
    error_failure(
        is_blocked,
        "HTTPException",
        "GraySwanGuardrailAPIError",
        error.to_string(),
    )
}

fn failure_with_class(class_name: &str, message: String) -> Box<crate::helpers::HelperError> {
    let class = class_name.to_owned();
    Box::new(crate::helpers::HelperError {
        class,
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn guardrail_error<E>(error: E) -> Box<crate::helpers::HelperError>
where
    E: std::fmt::Display,
{
    let message = error.to_string();
    failure_with_class("GuardrailError", message)
}

fn field_or_empty_object(input: &serde_json::Value, key: &str) -> serde_json::Value {
    input.get(key).cloned().unwrap_or_else(|| json!({}))
}

const fn build_grayswan_input(
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

fn current_thread_runtime() -> Result<tokio::runtime::Runtime, Box<crate::helpers::HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(guardrail_error)
}
