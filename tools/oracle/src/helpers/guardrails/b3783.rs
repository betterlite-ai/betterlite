use std::collections::BTreeMap;
use std::net::TcpListener;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::onyx::{OnyxGuardrail, OnyxGuardrailError, SUPPORTED_EVENT_HOOKS};
use betterlite_http::proxy::base_builder;
use secrecy::SecretString;
use serde_json::json;

use crate::case::UpstreamAnswer;
use crate::helpers::{HelperError, HelperResult};
use crate::mock::MockProvider;
use crate::routing_mock_scripts::MockScript;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OnyxCall {
    settings: Settings,
    #[serde(default)]
    inputs: BTreeMap<String, GuardrailValue>,
    #[serde(default)]
    request_data: BTreeMap<String, GuardrailValue>,
    input_type: GuardrailInputType,
    litellm_call_id: Option<String>,
    answer: Option<UpstreamAnswer>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    api_key: Option<String>,
    api_base: Option<String>,
    timeout: Option<f64>,
}

impl crate::helpers::Helper for Op {
    type Input = OnyxCall;

    fn run(input: &Self::Input) -> HelperResult {
        call(input)
    }
}

type Prepared = (Option<MockProvider>, TcpListener, String);
type Recorded = (Option<String>, Option<serde_json::Value>, Option<String>);

fn call(input: &OnyxCall) -> HelperResult {
    let (mock, silent, base) = start_mock(input.answer.as_ref())?;
    let guardrail = build_guardrail(input, &base)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("OSError", None, error.to_string()))?;
    let result = runtime.block_on(guardrail.apply_guardrail(
        input.inputs.clone(),
        &input.request_data,
        input.input_type,
        input.litellm_call_id.as_deref(),
    ));
    drop(silent);
    let (path, request, content_type) = finish(mock)?;
    match result {
        Ok(inputs) => Ok(json!({
            "supported_event_hooks": SUPPORTED_EVENT_HOOKS,
            "path": path,
            "request": request,
            "content_type": content_type,
            "inputs": inputs,
        })),
        Err(error @ OnyxGuardrailError::Blocked { .. }) => Ok(json!({
            "blocked": true,
            "status": error.status_code().as_u16(),
            "path": path,
            "request": request,
            "content_type": content_type,
        })),
        Err(error) => Err(onyx_failure(&error)),
    }
}

fn start_mock(answer: Option<&UpstreamAnswer>) -> Result<Prepared, Box<HelperError>> {
    let io = |error: std::io::Error| failure("OSError", None, error.to_string());
    // with no answer the listener accepts the connection and never replies, so the call times out
    let silent = TcpListener::bind(("127.0.0.1", 0)).map_err(io)?;
    let mock = answer
        .map(|reply| {
            MockScript::try_from(std::slice::from_ref(reply)).and_then(MockProvider::start)
        })
        .transpose()
        .map_err(|error| failure("MockError", None, error.to_string()))?;
    let base = match &mock {
        Some(provider) => provider.base_url.clone(),
        None => format!("http://{}", silent.local_addr().map_err(io)?),
    };
    Ok((mock, silent, base))
}

fn finish(mock: Option<MockProvider>) -> Result<Recorded, Box<HelperError>> {
    let calls = mock
        .map(MockProvider::finish)
        .transpose()
        .map_err(|error| failure("MockError", None, error.to_string()))?;
    let recorded = calls.unwrap_or_default().into_iter().next();
    let path = recorded.as_ref().map(|call| call.request.path.clone());
    let content_type = recorded
        .as_ref()
        .and_then(|call| call.request.headers.get("content-type").cloned());
    let request = recorded.map(|call| call.request.body);
    Ok((path, request, content_type))
}

fn build_guardrail(input: &OnyxCall, base: &str) -> Result<OnyxGuardrail, Box<HelperError>> {
    let mut guardrail = OnyxGuardrail::new(
        input.settings.api_key.clone().map(SecretString::from),
        input.settings.api_base.clone(),
        input.settings.timeout,
        &BTreeMap::new(),
        base_builder(),
    )
    .map_err(|error| onyx_failure(&error))?;
    base.clone_into(&mut guardrail.api_base);
    Ok(guardrail)
}

fn onyx_failure(error: &OnyxGuardrailError) -> Box<HelperError> {
    let class = match error {
        OnyxGuardrailError::Blocked { .. } => "HTTPException",
        OnyxGuardrailError::MissingApiKey
        | OnyxGuardrailError::TimeoutNotInteger(..)
        | OnyxGuardrailError::TimeoutNegative(..)
        | OnyxGuardrailError::UnrepresentableTimeout { .. } => "ValueError",
        OnyxGuardrailError::Transport { .. }
        | OnyxGuardrailError::HttpStatus { .. }
        | OnyxGuardrailError::ResponseTooLarge { .. }
        | OnyxGuardrailError::ResponseJson { .. } => "OnyxGuardrailAPIError",
    };
    let status =
        matches!(error, OnyxGuardrailError::Blocked { .. }).then(|| error.status_code().as_u16());
    failure(class, status, error.to_string())
}

fn failure(class_name: &str, status: Option<u16>, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class_name.to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
