use std::collections::BTreeMap;
use std::fmt::Display;
use std::net::TcpListener;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::aporia_guardrail::error::AporiaGuardrailError;
use betterlite_guardrails::aporia_guardrail::{
    AporiaGuardrail, ModerationGate, Settings as AporiaSettings,
};
use betterlite_guardrails::generic_guardrail_contracts::GuardrailMessage;
use betterlite_guardrails::guardrails_ai_guardrail::error::GuardrailsAiGuardrailError;
use betterlite_guardrails::guardrails_ai_guardrail::{
    GuardrailsAiGuardrail, InputFormat, Settings as GuardrailsAiSettings,
};
use secrecy::SecretString;
use serde_json::json;

use crate::case::UpstreamAnswer;
use crate::helpers::{HelperError, HelperResult};
use crate::mock::MockProvider;
use crate::routing_mock_scripts::MockScript;

mod request;
use request::{
    AporiaModerationRequest, AporiaPostCallRequest, GuardrailsAiConstructRequest,
    GuardrailsAiProcessRequest,
};

pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    AporiaModeration(AporiaModerationRequest),
    AporiaPostCall(AporiaPostCallRequest),
    GuardrailsAiConstruct(GuardrailsAiConstructRequest),
    #[serde(rename = "guardrails_ai_process_input")]
    GuardrailsAiProcess(GuardrailsAiProcessRequest),
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::AporiaModeration(request) => run_aporia(
                &request.api_key,
                &request.dynamic_params,
                request.answer.as_ref(),
                AporiaCall::Moderation(request.messages.clone()),
            ),
            Input::AporiaPostCall(request) => run_aporia(
                &request.api_key,
                &request.dynamic_params,
                request.answer.as_ref(),
                AporiaCall::PostCall(request.messages.clone(), request.response_string.clone()),
            ),
            Input::GuardrailsAiConstruct(request) => guardrails_ai_construct(
                request.guard_name.clone(),
                request.api_base.clone(),
                &request.environment,
            ),
            Input::GuardrailsAiProcess(request) => guardrails_ai_process_input(
                &request.guard_name,
                request.input_format,
                request.last_user_message.clone(),
                &request.dynamic_params,
                request.answer.as_ref(),
            ),
        }
    }
}

type Prepared = (
    Option<MockProvider>,
    TcpListener,
    String,
    tokio::runtime::Runtime,
);

fn prepare(answer: Option<&UpstreamAnswer>) -> Result<Prepared, Box<HelperError>> {
    let io = |error: std::io::Error| fail("OSError", error.to_string());
    // with no answer this stays bound and silent until apply drops it, so the call times out instead of being refused
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

fn aporia_guardrail(api_key: &str, base: &str) -> AporiaGuardrail {
    AporiaGuardrail {
        settings: AporiaSettings {
            api_key: SecretString::from(api_key.to_owned()),
            api_base: base.to_owned(),
        },
        client: reqwest::Client::new(),
        timeout: Duration::from_secs(30),
    }
}

type AporiaPrepared = (
    AporiaGuardrail,
    TcpListener,
    tokio::runtime::Runtime,
    Option<MockProvider>,
);

fn prepare_aporia(
    api_key: &str,
    answer: Option<&UpstreamAnswer>,
) -> Result<AporiaPrepared, Box<HelperError>> {
    let (mock, silent, base, runtime) = prepare(answer)?;
    Ok((aporia_guardrail(api_key, &base), silent, runtime, mock))
}

enum AporiaCall {
    Moderation(Option<Vec<GuardrailMessage>>),
    PostCall(Vec<GuardrailMessage>, Option<String>),
}

fn run_aporia(
    api_key: &str,
    dynamic_params: &BTreeMap<String, GuardrailValue>,
    answer: Option<&UpstreamAnswer>,
    call: AporiaCall,
) -> HelperResult {
    let (guardrail, silent, runtime, mock) = prepare_aporia(api_key, answer)?;
    let result = runtime.block_on(async {
        match call {
            AporiaCall::Moderation(messages) => {
                let gate = ModerationGate {
                    should_run: true,
                    should_proceed: true,
                };
                guardrail
                    .async_moderation_hook(gate, messages, dynamic_params)
                    .await
            }
            AporiaCall::PostCall(messages, response_string) => {
                guardrail
                    .async_post_call_success_hook(true, response_string, messages, dynamic_params)
                    .await
            }
        }
    });
    drop(silent);
    finish_aporia(mock, result)
}

fn finish_aporia(
    mock: Option<MockProvider>,
    result: Result<bool, AporiaGuardrailError>,
) -> HelperResult {
    finish(
        mock,
        result,
        |error| match error {
            AporiaGuardrailError::Blocked { .. } => "HTTPException",
            AporiaGuardrailError::MissingEnvironment { .. } => "KeyError",
            AporiaGuardrailError::Timeout { .. }
            | AporiaGuardrailError::Request { .. }
            | AporiaGuardrailError::InvalidHeader { .. }
            | AporiaGuardrailError::HttpStatus { .. }
            | AporiaGuardrailError::ResponseTooLarge { .. }
            | AporiaGuardrailError::ResponseJson { .. } => "AporiaGuardrailAPIError",
        },
        |path, request, _ran| json!({ "path": path, "request": request }),
    )
}

fn extract_call(
    mock: Option<MockProvider>,
) -> Result<(Option<String>, Option<serde_json::Value>), Box<HelperError>> {
    let calls = mock
        .map(MockProvider::finish)
        .transpose()
        .map_err(|error| fail("MockError", error.to_string()))?;
    let recorded = calls.unwrap_or_default().into_iter().next();
    let path = recorded.as_ref().map(|call| call.request.path.clone());
    let request = recorded.map(|call| call.request.body);
    Ok((path, request))
}

fn finish<T, E, Classify, Build>(
    mock: Option<MockProvider>,
    result: Result<T, E>,
    classify: Classify,
    build: Build,
) -> HelperResult
where
    E: Display,
    Classify: FnOnce(&E) -> &'static str,
    Build: FnOnce(Option<String>, Option<serde_json::Value>, T) -> serde_json::Value,
{
    let (path, request) = extract_call(mock)?;
    match result {
        Ok(value) => Ok(build(path, request, value)),
        Err(error) => Err(fail(classify(&error), error.to_string())),
    }
}

fn guardrails_ai_construct(
    guard_name: Option<String>,
    api_base: Option<String>,
    environment: &BTreeMap<String, String>,
) -> HelperResult {
    let settings =
        GuardrailsAiSettings::resolve(guard_name, api_base, InputFormat::LlmOutput, environment)
            .map_err(|error| fail(classify_guardrails_ai(&error), error.to_string()))?;
    Ok(json!({
        "guard_name": settings.guard_name,
        "api_base": settings.api_base,
        "input_format": settings.input_format,
    }))
}

fn guardrails_ai_process_input(
    guard_name: &str,
    input_format: InputFormat,
    last_user_message: Option<String>,
    dynamic_params: &BTreeMap<String, GuardrailValue>,
    answer: Option<&UpstreamAnswer>,
) -> HelperResult {
    let (mock, silent, base, runtime) = prepare(answer)?;
    let guardrail = GuardrailsAiGuardrail {
        settings: GuardrailsAiSettings {
            guard_name: guard_name.to_owned(),
            api_base: base,
            input_format,
        },
        client: reqwest::Client::new(),
        timeout: Duration::from_secs(30),
    };
    let result = runtime.block_on(guardrail.process_input(last_user_message, dynamic_params));
    drop(silent);
    finish_guardrails_ai(mock, result)
}

fn finish_guardrails_ai(
    mock: Option<MockProvider>,
    result: Result<Option<String>, GuardrailsAiGuardrailError>,
) -> HelperResult {
    finish(
        mock,
        result,
        classify_guardrails_ai,
        |path, request, updated_text| json!({ "path": path, "request": request, "updated_text": updated_text }),
    )
}

const fn classify_guardrails_ai(error: &GuardrailsAiGuardrailError) -> &'static str {
    match error {
        GuardrailsAiGuardrailError::MissingGuardName => "Exception",
        GuardrailsAiGuardrailError::Blocked { .. }
        | GuardrailsAiGuardrailError::PreCallBlocked { .. } => "HTTPException",
        GuardrailsAiGuardrailError::Timeout { .. }
        | GuardrailsAiGuardrailError::Request { .. }
        | GuardrailsAiGuardrailError::HttpStatus { .. }
        | GuardrailsAiGuardrailError::ResponseTooLarge { .. }
        | GuardrailsAiGuardrailError::ResponseJson { .. }
        | GuardrailsAiGuardrailError::PreCallMissingOutput
        | GuardrailsAiGuardrailError::InvalidUtf8 { .. } => "GuardrailsAIException",
    }
}
