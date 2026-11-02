use std::net::TcpListener;

use serde_json::json;

use crate::case::UpstreamAnswer;
use crate::helpers::{HelperError, HelperResult};
use crate::mock::MockProvider;
use crate::routing_mock_scripts::MockScript;

mod aporia;
mod guardrails_ai;
mod request;
use aporia::AporiaCall;
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
            Input::AporiaModeration(request) => aporia::run_aporia(
                &request.api_key,
                &request.dynamic_params,
                request.answer.as_ref(),
                AporiaCall::Moderation(request.messages.clone()),
            ),
            Input::AporiaPostCall(request) => aporia::run_aporia(
                &request.api_key,
                &request.dynamic_params,
                request.answer.as_ref(),
                AporiaCall::PostCall(request.messages.clone(), request.response_string.clone()),
            ),
            Input::GuardrailsAiConstruct(request) => guardrails_ai::guardrails_ai_construct(
                request.guard_name.clone(),
                request.api_base.clone(),
                &request.environment,
            ),
            Input::GuardrailsAiProcess(request) => {
                guardrails_ai::guardrails_ai_process_input(request)
            }
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

trait OracleBlockable {
    fn blocked_response(&self) -> Option<serde_json::Value>;
    fn status_u16(&self) -> u16;
    fn classify(&self) -> &'static str;
}

fn finish<T, E, F>(mock: Option<MockProvider>, result: Result<T, E>, on_ok: F) -> HelperResult
where
    E: OracleBlockable + ToString,
    F: FnOnce(&mut serde_json::Map<String, serde_json::Value>, T),
{
    let (path, request) = extract_call(mock)?;
    match result {
        Ok(value) => {
            let mut body = serde_json::Map::new();
            body.insert("path".to_owned(), json!(path));
            body.insert("request".to_owned(), json!(request));
            on_ok(&mut body, value);
            Ok(serde_json::Value::Object(body))
        }
        Err(error) => error.blocked_response().map_or_else(
            || Err(fail(error.classify(), error.to_string())),
            |response| {
                Ok(json!({
                    "path": path,
                    "request": request,
                    "blocked": true,
                    "status": error.status_u16(),
                    "response": response,
                }))
            },
        ),
    }
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
