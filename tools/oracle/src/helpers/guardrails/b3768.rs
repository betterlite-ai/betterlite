use std::collections::BTreeMap;

use betterlite_config::loader::ConfigError;
use betterlite_config::sections::dynamoai_moderation::Parameters;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::dynamoai::decisions::GuardrailStatus;
use betterlite_guardrails::dynamoai::error::DynamoAiError;
use betterlite_guardrails::dynamoai::guardrail::DynamoAiGuardrail;
use betterlite_guardrails::dynamoai::hooks::{
    EventHook, PostCallOutcome, RequestData, ResponseChoice, ResponseMessage, SUPPORTED_EVENT_HOOKS,
};
use betterlite_http::proxy::base_builder;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::case::UpstreamAnswer;
use crate::helpers::{HelperError, HelperResult};
use crate::mock::MockProvider;
use crate::routing_mock_scripts::MockScript;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    mode: Mode,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    policy_ids: Option<Vec<String>>,
    #[serde(default)]
    model_id: Option<String>,
    #[serde(default)]
    messages: Vec<GuardrailValue>,
    #[serde(default)]
    choices: Vec<CaseChoice>,
    #[serde(default)]
    answer: Option<UpstreamAnswer>,
}

#[derive(Deserialize)]
struct CaseChoice {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    content: Option<GuardrailValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Constructor,
    PreCall,
    DuringCall,
    PostCall,
}

#[derive(Serialize)]
struct RequestRecord {
    path: String,
    content_type: Option<String>,
    authorization_present: bool,
    body: Value,
}

#[derive(Serialize)]
struct Output {
    created: bool,
    api_base: Option<String>,
    policy_ids: Vec<String>,
    model_id: String,
    event_hooks: Vec<&'static str>,
    requests: Vec<RequestRecord>,
    status: Option<&'static str>,
    raised: bool,
    error: Option<&'static str>,
}

impl crate::helpers::Helper for Op {
    type Input = Call;

    fn run(input: &Self::Input) -> HelperResult {
        run_call(input)
    }
}

fn run_call(call: &Call) -> HelperResult {
    let params: Parameters = serde_json::from_value(serde_json::json!({
        "guardrail": "dynamoai",
        "api_key": call.api_key,
        "api_base": call.api_base,
        "policy_ids": call.policy_ids,
        "model_id": call.model_id,
    }))
    .map_err(|error| failure("ValueError", error.to_string()))?;
    let mut settings = match params.resolve("dynamoai", "guardrail", &call.env) {
        Ok(settings) => settings,
        Err(ConfigError::MissingEnvironment { .. }) => {
            return serde_json::to_value(Output {
                created: false,
                api_base: None,
                policy_ids: Vec::new(),
                model_id: String::new(),
                event_hooks: Vec::new(),
                requests: Vec::new(),
                status: None,
                raised: false,
                error: Some("missing_api_key"),
            })
            .map_err(|error| failure("ValueError", error.to_string()));
        }
        Err(error) => return Err(failure("ValueError", error.to_string())),
    };
    let api_base = settings.api_base.clone();
    if call.mode == Mode::Constructor {
        return serde_json::to_value(Output {
            created: true,
            api_base: Some(api_base),
            policy_ids: settings.policy_ids.clone(),
            model_id: settings.model_id.clone(),
            event_hooks: event_hook_names(),
            requests: Vec::new(),
            status: None,
            raised: false,
            error: None,
        })
        .map_err(|error| failure("ValueError", error.to_string()));
    }
    let client = base_builder()
        .build()
        .map_err(|error| failure("RequestError", error.to_string()))?;
    let mock = MockProvider::start(script(call)?)
        .map_err(|error| failure("MockError", error.to_string()))?;
    settings.api_base.clone_from(&mock.base_url);
    let guardrail = DynamoAiGuardrail { settings, client };
    let result = build_runtime()?.block_on(run_mode(call, &guardrail));
    let requests: Vec<RequestRecord> = mock
        .finish()
        .map_err(|error| failure("MockError", error.to_string()))?
        .into_iter()
        .map(|received| RequestRecord {
            path: received.request.path,
            content_type: received.request.headers.get("content-type").cloned(),
            authorization_present: received.request.headers.contains_key("authorization"),
            body: received.request.body,
        })
        .collect();
    let (status, raised) = if requests.is_empty() {
        (None, false)
    } else {
        match &result {
            Ok(Some(status)) => (Some(status.as_str()), false),
            Ok(None) => (None, false),
            Err(DynamoAiError::Blocked { .. }) => {
                (Some(GuardrailStatus::Intervened.as_str()), true)
            }
            Err(_error) => (Some(GuardrailStatus::FailedToRespond.as_str()), true),
        }
    };
    serde_json::to_value(Output {
        created: true,
        api_base: Some(api_base),
        policy_ids: guardrail.settings.policy_ids.clone(),
        model_id: guardrail.settings.model_id.clone(),
        event_hooks: event_hook_names(),
        requests,
        status,
        raised,
        error: None,
    })
    .map_err(|error| failure("ValueError", error.to_string()))
}

fn event_hook_names() -> Vec<&'static str> {
    SUPPORTED_EVENT_HOOKS
        .iter()
        .map(|hook| match hook {
            EventHook::PreCall => "pre_call",
            EventHook::PostCall => "post_call",
            EventHook::DuringCall => "during_call",
        })
        .collect()
}

async fn run_mode(
    call: &Call,
    guardrail: &DynamoAiGuardrail,
) -> Result<Option<GuardrailStatus>, DynamoAiError> {
    let choices: Vec<ResponseChoice> = call
        .choices
        .iter()
        .map(|choice| ResponseChoice {
            message: ResponseMessage {
                role: choice.role.clone(),
                content: choice.content.clone(),
            },
        })
        .collect();
    match call.mode {
        Mode::PreCall => guardrail
            .async_pre_call_hook(&RequestData {
                messages: call.messages.clone(),
            })
            .await
            .map(Some),
        Mode::DuringCall => guardrail
            .async_moderation_hook(&call.messages)
            .await
            .map(Some),
        Mode::PostCall => match guardrail.async_post_call_success_hook(&choices).await? {
            PostCallOutcome::NoText => Ok(None),
            PostCallOutcome::Inspected(status) => Ok(Some(status)),
        },
        Mode::Constructor => Ok(None),
    }
}

fn script(call: &Call) -> Result<MockScript, Box<HelperError>> {
    let answer = call
        .answer
        .as_ref()
        .ok_or_else(|| failure("MockError", "the case has no answer".to_owned()))?;
    MockScript::try_from(std::slice::from_ref(answer))
        .map_err(|error| failure("MockError", error.to_string()))
}

fn build_runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("RuntimeError", error.to_string()))
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
