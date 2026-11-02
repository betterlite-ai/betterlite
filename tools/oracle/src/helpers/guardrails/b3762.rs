use std::net::TcpListener;

use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_config::sections::cisco_verdict_contracts::Configured;
use betterlite_guardrails::cisco_configuration::records::Direction;
use betterlite_guardrails::cisco_conversation::Fields;
use betterlite_guardrails::cisco_conversation::json::Json;
use betterlite_guardrails::cisco_conversation::request::request_messages;
use betterlite_guardrails::cisco_inspection::chat::{ChatInspectionArgs, inspect_chat};
use betterlite_guardrails::cisco_inspection::construct::CiscoInspectionGuardrail;
use betterlite_guardrails::cisco_inspection::hooks::{
    HookArgs, PostCallArgs, async_moderation_hook, async_post_call_success_hook,
    async_pre_call_hook,
};
use betterlite_guardrails::cisco_inspection::mcp::{inspect_mcp_request, inspect_mcp_response};
use betterlite_guardrails::cisco_inspection::transport::{build_headers, post_inspection};
use betterlite_guardrails::cisco_verdict_enforcement::api_error::ApiErrorOutcome;
use betterlite_guardrails::cisco_verdict_enforcement::finalize::{
    FinalizeOutcome, ResponseSurface,
};
use serde_json::{Value, json};

use crate::case::UpstreamAnswer;
use crate::helpers::{HelperError, HelperResult};
use crate::mock::MockProvider;
use crate::routing_mock_scripts::MockScript;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreCallInput {
    settings: Value,
    request_data: Value,
    #[serde(default)]
    call_type: Option<String>,
    #[serde(default = "default_true")]
    should_run: bool,
    #[serde(default)]
    key_user_id: Option<String>,
    answer: Option<UpstreamAnswer>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PostCallInput {
    settings: Value,
    request_data: Value,
    response: Value,
    #[serde(default = "default_true")]
    should_run: bool,
    #[serde(default)]
    key_user_id: Option<String>,
    answer: Option<UpstreamAnswer>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChatInput {
    settings: Value,
    request_data: Value,
    #[serde(default)]
    direction: Option<String>,
    #[serde(default)]
    key_user_id: Option<String>,
    answer: Option<UpstreamAnswer>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct McpRequestInput {
    settings: Value,
    request_data: Value,
    answer: Option<UpstreamAnswer>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct McpResponseInput {
    settings: Value,
    request_data: Value,
    response: Value,
    answer: Option<UpstreamAnswer>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PostInspectionInput {
    settings: Value,
    #[serde(default)]
    payload: Value,
    #[serde(default = "default_surface")]
    surface: String,
    answer: Option<UpstreamAnswer>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Construct { settings: Value },
    BuildHeaders { settings: Value },
    PreCallHook(PreCallInput),
    ModerationHook(PreCallInput),
    PostCallHook(PostCallInput),
    InspectChat(ChatInput),
    InspectMcpRequest(McpRequestInput),
    InspectMcpResponse(McpResponseInput),
    PostInspection(PostInspectionInput),
}

const fn default_true() -> bool {
    true
}

fn default_surface() -> String {
    "chat".to_owned()
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Construct { settings } => construct(settings),
            Input::BuildHeaders { settings } => headers(settings),
            Input::PreCallHook(args) => pre_call_hook(args, false),
            Input::ModerationHook(args) => pre_call_hook(args, true),
            Input::PostCallHook(args) => post_call_hook(args),
            Input::InspectChat(args) => chat(args),
            Input::InspectMcpRequest(args) => mcp_request(args),
            Input::InspectMcpResponse(args) => mcp_response(args),
            Input::PostInspection(args) => post_inspection_op(args),
        }
    }
}

fn fail(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn json_to_value(value: &Json) -> Value {
    match value {
        Json::Null => Value::Null,
        Json::Bool(flag) => Value::Bool(*flag),
        Json::Number(text) => serde_json::from_str(text).unwrap_or(Value::Null),
        Json::Text(text) => Value::String(text.clone()),
        Json::List(items) => Value::Array(
            items
                .iter()
                .map(|raw| serde_json::from_str(raw.get()).unwrap_or(Value::Null))
                .collect(),
        ),
        Json::Object(fields) => {
            serde_json::from_str(&serde_json::to_string(fields).unwrap_or_default())
                .unwrap_or(Value::Null)
        }
    }
}

fn configure(settings: &Value) -> Result<Configured, Box<HelperError>> {
    let mut parameters = settings.clone();
    if let Value::Object(map) = &mut parameters {
        map.insert("guardrail".to_owned(), json!("cisco_ai_defense"));
    }
    let entry = json!({"guardrail_name": "cisco-ai-defense", "litellm_params": parameters});
    let loaded = ConfigLoader {
        lookup: std::collections::BTreeMap::<String, String>::new(),
    }
    .load(&json!({"guardrails": [entry]}).to_string());
    let outcome = loaded.map(|config| {
        config
            .sections
            .cisco_verdict_contracts
            .guardrails
            .into_iter()
            .next()
    });
    let (class, message) = match outcome {
        Ok(Some(configured)) => return Ok(configured),
        Ok(None) => ("ConfigError", "no Cisco AI Defense guardrail".to_owned()),
        Err(error) => {
            let class = match error {
                ConfigError::MissingEnvironment { .. } => "CiscoAIDefenseGuardrailMissingSecrets",
                ConfigError::Yaml { .. }
                | ConfigError::Unsupported { .. }
                | ConfigError::InvalidFloat { .. }
                | ConfigError::InvalidInteger { .. }
                | ConfigError::InvalidScalar { .. } => "ConfigError",
            };
            (class, error.to_string())
        }
    };
    Err(fail(class, message))
}

fn guardrail(settings: &Value) -> Result<CiscoInspectionGuardrail, Box<HelperError>> {
    let configured = configure(settings)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| fail("OSError", error.to_string()))?;
    Ok(CiscoInspectionGuardrail {
        settings: configured,
        client,
    })
}

fn construct(settings: &Value) -> HelperResult {
    let configured = configure(settings)?;
    Ok(json!({
        "guardrail_name": configured.guardrail_name,
        "api_base": configured.api_base,
        "inspect_path": configured.inspect_path,
        "inspection_type": match configured.inspection_type {
            betterlite_config::sections::cisco_verdict_contracts::InspectionType::Chat => "chat",
            betterlite_config::sections::cisco_verdict_contracts::InspectionType::Mcp => "mcp",
        },
        "on_flagged_action": match configured.on_flagged_action {
            betterlite_config::sections::cisco_verdict_contracts::FlaggedAction::Block => "block",
            betterlite_config::sections::cisco_verdict_contracts::FlaggedAction::Monitor => "monitor",
        },
        "fallback_on_error": match configured.fallback_on_error {
            betterlite_config::sections::cisco_verdict_contracts::FallbackAction::Allow => "allow",
            betterlite_config::sections::cisco_verdict_contracts::FallbackAction::Block => "block",
        },
        "timeout_seconds": configured.timeout,
    }))
}

fn headers(settings: &Value) -> HelperResult {
    let guardrail = guardrail(settings)?;
    let map = build_headers(&guardrail)
        .map_err(|error| fail("CiscoAIDefenseGuardrailAPIError", error.0))?;
    let mut rendered = std::collections::BTreeMap::new();
    for (name, value) in &map {
        let text = value
            .to_str()
            .map_err(|error| fail("OSError", error.to_string()))?;
        rendered.insert(name.as_str().to_owned(), text.to_owned());
    }
    Ok(json!({ "headers": rendered }))
}

type Prepared = (
    Option<MockProvider>,
    TcpListener,
    String,
    tokio::runtime::Runtime,
);

fn prepare(answer: Option<&UpstreamAnswer>) -> Result<Prepared, Box<HelperError>> {
    let io = |error: std::io::Error| fail("OSError", error.to_string());
    // with no answer this stays bound and silent until the caller drops it, so the call times out instead of being refused
    let silent = TcpListener::bind(("127.0.0.1", 0)).map_err(io)?;
    let mock = answer
        .map(|reply| {
            MockScript::try_from(std::slice::from_ref(reply)).and_then(MockProvider::start)
        })
        .transpose()
        .map_err(|error| fail("MockError", error.to_string()))?;
    let base = match &mock {
        Some(provider) => provider.base_url.clone(),
        None => format!("http://{}", silent.local_addr().map_err(io)?),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(io)?;
    Ok((mock, silent, base, runtime))
}

fn recorded_call(
    mock: Option<MockProvider>,
) -> Result<(Option<String>, Option<Value>), Box<HelperError>> {
    let calls = mock
        .map(MockProvider::finish)
        .transpose()
        .map_err(|error| fail("MockError", error.to_string()))?;
    let recorded = calls.unwrap_or_default().into_iter().next();
    let path = recorded.as_ref().map(|call| call.request.path.clone());
    let body = recorded.map(|call| call.request.body);
    Ok((path, body))
}

fn outcome_tag(outcome: &FinalizeOutcome) -> Value {
    match outcome {
        FinalizeOutcome::Allow => json!({"tag": "allow"}),
        FinalizeOutcome::Redacted(_) => json!({"tag": "redacted"}),
        FinalizeOutcome::Monitored => json!({"tag": "monitored"}),
        FinalizeOutcome::Blocked(_) => json!({"tag": "blocked"}),
        FinalizeOutcome::ApiError(ApiErrorOutcome::Unscanned) => json!({"tag": "unscanned"}),
        FinalizeOutcome::ApiError(ApiErrorOutcome::Blocked(detail)) => json!({
            "tag": "api_error_blocked",
            "detail": {
                "error": detail.error,
                "message": detail.message,
                "error_type": detail.error_type,
            },
        }),
    }
}

fn applied_guardrails(request_data: &Fields) -> Vec<String> {
    let bucket_key = if request_data.contains_key("litellm_metadata") {
        "litellm_metadata"
    } else {
        "metadata"
    };
    let Json::Object(bucket) = Json::field(request_data, bucket_key) else {
        return Vec::new();
    };
    match Json::field(&bucket, "applied_guardrails") {
        Json::List(items) => items
            .iter()
            .filter_map(|item| match Json::parse(item.get()) {
                Json::Text(name) => Some(name),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    None
                }
            })
            .collect(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            Vec::new()
        }
    }
}

fn pre_call_hook(args: &PreCallInput, is_moderation: bool) -> HelperResult {
    let (mock, silent, base, runtime) = prepare(args.answer.as_ref())?;
    let mut guardrail = guardrail(&args.settings)?;
    guardrail.settings.api_base = base;
    let mut data = serde_json::from_value(args.request_data.clone()).unwrap_or_default();
    let hook_args = HookArgs {
        call_type: args.call_type.as_deref(),
        should_run: args.should_run,
        key_user_id: args.key_user_id.as_deref(),
    };
    let outcome = if is_moderation {
        runtime.block_on(async_moderation_hook(&guardrail, &mut data, hook_args))
    } else {
        runtime.block_on(async_pre_call_hook(&guardrail, &mut data, hook_args))
    };
    drop(silent);
    let (path, body) = recorded_call(mock)?;
    Ok(json!({
        "path": path,
        "request": body,
        "outcome": outcome.as_ref().map(outcome_tag),
        "applied_guardrails": applied_guardrails(&data),
    }))
}

fn post_call_hook(args: &PostCallInput) -> HelperResult {
    let (mock, silent, base, runtime) = prepare(args.answer.as_ref())?;
    let mut guardrail = guardrail(&args.settings)?;
    guardrail.settings.api_base = base;
    let mut data = serde_json::from_value(args.request_data.clone()).unwrap_or_default();
    let mut response: Fields = serde_json::from_value(args.response.clone()).unwrap_or_default();
    let hook_args = PostCallArgs {
        should_run: args.should_run,
        key_user_id: args.key_user_id.as_deref(),
    };
    let outcome = runtime.block_on(async_post_call_success_hook(
        &guardrail,
        &mut data,
        &mut response,
        hook_args,
    ));
    drop(silent);
    let (path, body) = recorded_call(mock)?;
    Ok(json!({
        "path": path,
        "request": body,
        "outcome": outcome.as_ref().map(outcome_tag),
        "applied_guardrails": applied_guardrails(&data),
        "response": serde_json::from_str(&serde_json::to_string(&response).unwrap_or_default())
            .unwrap_or(Value::Null),
    }))
}

fn chat(args: &ChatInput) -> HelperResult {
    let (mock, silent, base, runtime) = prepare(args.answer.as_ref())?;
    let mut guardrail = guardrail(&args.settings)?;
    guardrail.settings.api_base = base;
    let mut data = serde_json::from_value(args.request_data.clone()).unwrap_or_default();
    let messages = request_messages(&data);
    let direction = match args.direction.as_deref() {
        Some("output") => Direction::Output,
        _ => Direction::Input,
    };
    let chat_args = ChatInspectionArgs {
        messages,
        direction,
        key_user_id: args.key_user_id.as_deref(),
    };
    let outcome = runtime.block_on(inspect_chat(
        &guardrail,
        chat_args,
        &mut data,
        ResponseSurface::None,
    ));
    drop(silent);
    let (path, body) = recorded_call(mock)?;
    Ok(json!({
        "path": path,
        "request": body,
        "outcome": outcome_tag(&outcome),
    }))
}

fn mcp_request(args: &McpRequestInput) -> HelperResult {
    let (mock, silent, base, runtime) = prepare(args.answer.as_ref())?;
    let mut guardrail = guardrail(&args.settings)?;
    guardrail.settings.api_base = base;
    let mut data = serde_json::from_value(args.request_data.clone()).unwrap_or_default();
    let outcome = runtime.block_on(inspect_mcp_request(&guardrail, &mut data));
    drop(silent);
    let (path, body) = recorded_call(mock)?;
    Ok(json!({
        "path": path,
        "request": body,
        "outcome": outcome.as_ref().map(outcome_tag),
    }))
}

fn mcp_response(args: &McpResponseInput) -> HelperResult {
    let (mock, silent, base, runtime) = prepare(args.answer.as_ref())?;
    let mut guardrail = guardrail(&args.settings)?;
    guardrail.settings.api_base = base;
    let mut data = serde_json::from_value(args.request_data.clone()).unwrap_or_default();
    let response_json = Json::parse(&args.response.to_string());
    let outcome = runtime.block_on(inspect_mcp_response(&guardrail, &mut data, &response_json));
    drop(silent);
    let (path, body) = recorded_call(mock)?;
    Ok(json!({
        "path": path,
        "request": body,
        "outcome": outcome.as_ref().map(outcome_tag),
    }))
}

fn post_inspection_op(args: &PostInspectionInput) -> HelperResult {
    let (mock, silent, base, runtime) = prepare(args.answer.as_ref())?;
    let mut guardrail = guardrail(&args.settings)?;
    guardrail.settings.api_base = base;
    let url = format!(
        "{}{}",
        guardrail.settings.api_base, guardrail.settings.inspect_path
    );
    let result = runtime.block_on(post_inspection(
        &guardrail,
        &url,
        &args.payload,
        &args.surface,
    ));
    drop(silent);
    let (path, body) = recorded_call(mock)?;
    let value = result
        .map(|json| json_to_value(&json))
        .map_err(|error| fail("CiscoAIDefenseGuardrailAPIError", error.0))?;
    Ok(json!({
        "path": path,
        "request": body,
        "result": value,
    }))
}

mod tests;
