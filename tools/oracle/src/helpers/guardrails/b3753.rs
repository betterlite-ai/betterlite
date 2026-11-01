use std::collections::BTreeMap;
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

pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    AporiaModeration {
        api_key: String,
        messages: Option<Vec<GuardrailMessage>>,
        dynamic_params: BTreeMap<String, GuardrailValue>,
        answer: Option<UpstreamAnswer>,
    },
    AporiaPostCall {
        api_key: String,
        messages: Vec<GuardrailMessage>,
        response_string: Option<String>,
        dynamic_params: BTreeMap<String, GuardrailValue>,
        answer: Option<UpstreamAnswer>,
    },
    GuardrailsAiConstruct {
        guard_name: Option<String>,
        api_base: Option<String>,
        environment: BTreeMap<String, String>,
    },
    #[serde(rename = "guardrails_ai_process_input")]
    GuardrailsAiProcess {
        guard_name: String,
        input_format: InputFormat,
        last_user_message: Option<String>,
        dynamic_params: BTreeMap<String, GuardrailValue>,
        answer: Option<UpstreamAnswer>,
    },
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::AporiaModeration {
                api_key,
                messages,
                dynamic_params,
                answer,
            } => aporia_moderation(api_key, messages.clone(), dynamic_params, answer.as_ref()),
            Input::AporiaPostCall {
                api_key,
                messages,
                response_string,
                dynamic_params,
                answer,
            } => aporia_post_call(
                api_key,
                messages.clone(),
                response_string.clone(),
                dynamic_params,
                answer.as_ref(),
            ),
            Input::GuardrailsAiConstruct {
                guard_name,
                api_base,
                environment,
            } => guardrails_ai_construct(guard_name.clone(), api_base.clone(), environment),
            Input::GuardrailsAiProcess {
                guard_name,
                input_format,
                last_user_message,
                dynamic_params,
                answer,
            } => guardrails_ai_process_input(
                guard_name,
                *input_format,
                last_user_message.clone(),
                dynamic_params,
                answer.as_ref(),
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

fn aporia_moderation(
    api_key: &str,
    messages: Option<Vec<GuardrailMessage>>,
    dynamic_params: &BTreeMap<String, GuardrailValue>,
    answer: Option<&UpstreamAnswer>,
) -> HelperResult {
    let (mock, silent, base, runtime) = prepare(answer)?;
    let guardrail = aporia_guardrail(api_key, &base);
    let gate = ModerationGate {
        should_run: true,
        should_proceed: true,
    };
    let result = runtime.block_on(guardrail.async_moderation_hook(gate, messages, dynamic_params));
    drop(silent);
    finish_aporia(mock, result)
}

fn aporia_post_call(
    api_key: &str,
    messages: Vec<GuardrailMessage>,
    response_string: Option<String>,
    dynamic_params: &BTreeMap<String, GuardrailValue>,
    answer: Option<&UpstreamAnswer>,
) -> HelperResult {
    let (mock, silent, base, runtime) = prepare(answer)?;
    let guardrail = aporia_guardrail(api_key, &base);
    let result = runtime.block_on(guardrail.async_post_call_success_hook(
        true,
        response_string,
        messages,
        dynamic_params,
    ));
    drop(silent);
    finish_aporia(mock, result)
}

fn finish_aporia(
    mock: Option<MockProvider>,
    result: Result<bool, AporiaGuardrailError>,
) -> HelperResult {
    let calls = mock
        .map(MockProvider::finish)
        .transpose()
        .map_err(|error| fail("MockError", error.to_string()))?;
    let recorded = calls.unwrap_or_default().into_iter().next();
    let path = recorded.as_ref().map(|call| call.request.path.clone());
    let request = recorded.map(|call| call.request.body);
    result.map_err(|error| aporia_failure(&error))?;
    Ok(json!({ "path": path, "request": request }))
}

fn aporia_failure(error: &AporiaGuardrailError) -> Box<HelperError> {
    let class = match error {
        AporiaGuardrailError::Blocked { .. } => "HTTPException",
        AporiaGuardrailError::MissingEnvironment { .. } => "KeyError",
        AporiaGuardrailError::Timeout { .. }
        | AporiaGuardrailError::Request { .. }
        | AporiaGuardrailError::InvalidHeader { .. }
        | AporiaGuardrailError::HttpStatus { .. }
        | AporiaGuardrailError::ResponseTooLarge { .. }
        | AporiaGuardrailError::ResponseJson { .. } => "AporiaGuardrailAPIError",
    };
    fail(class, error.to_string())
}

fn guardrails_ai_construct(
    guard_name: Option<String>,
    api_base: Option<String>,
    environment: &BTreeMap<String, String>,
) -> HelperResult {
    let settings =
        GuardrailsAiSettings::resolve(guard_name, api_base, InputFormat::LlmOutput, environment)
            .map_err(|error| guardrails_ai_failure(&error))?;
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
    };
    let result = runtime.block_on(guardrail.process_input(last_user_message, dynamic_params));
    drop(silent);
    finish_guardrails_ai(mock, result)
}

fn finish_guardrails_ai(
    mock: Option<MockProvider>,
    result: Result<Option<String>, GuardrailsAiGuardrailError>,
) -> HelperResult {
    let calls = mock
        .map(MockProvider::finish)
        .transpose()
        .map_err(|error| fail("MockError", error.to_string()))?;
    let recorded = calls.unwrap_or_default().into_iter().next();
    let path = recorded.as_ref().map(|call| call.request.path.clone());
    let request = recorded.map(|call| call.request.body);
    let updated_text = result.map_err(|error| guardrails_ai_failure(&error))?;
    Ok(json!({ "path": path, "request": request, "updated_text": updated_text }))
}

fn guardrails_ai_failure(error: &GuardrailsAiGuardrailError) -> Box<HelperError> {
    let class = match error {
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
    };
    fail(class, error.to_string())
}
