use std::collections::BTreeMap;

use betterlite_guardrails::akto::AktoMirroring;
use betterlite_guardrails::akto::error::AktoError;
use betterlite_guardrails::akto::payload::AktoPayload;
use betterlite_guardrails::akto::response::AktoAnswer;
use serde_json::{Value, json};

use crate::case::UpstreamAnswer;
use crate::helpers::HelperError;
use crate::mock::MockProvider;
use crate::mock::ProviderRequest;
use crate::routing_mock_scripts::MockScript;
use crate::routing_mock_scripts::ProviderCall;

use super::Input;
use super::support::failure;

pub(super) fn send(input: &Input, fire_and_forget: bool) -> crate::helpers::HelperResult {
    let mut guardrail = super::settings::build(input)?;
    let mock = MockProvider::start(script(&input.answer)?)
        .map_err(|error| failure("MockError", None, error.to_string()))?;
    guardrail.base_url.clone_from(&mock.base_url);
    let payload = guardrail
        .build_akto_payload(
            &super::akto_inputs(input),
            &input.request_data,
            input.status_code.unwrap_or(200),
            input.include_response,
        )
        .map_err(|error| failure("AktoError", None, error.to_string()))?;
    let answer = deliver(&guardrail, input, &payload, fire_and_forget)?;
    let calls = mock
        .finish()
        .map_err(|error| failure("MockError", None, error.to_string()))?;
    report(&guardrail, &calls, answer)
}

fn deliver(
    guardrail: &AktoMirroring,
    input: &Input,
    payload: &AktoPayload,
    fire_and_forget: bool,
) -> Result<Option<Result<AktoAnswer, AktoError>>, Box<HelperError>> {
    let guardrails = input.guardrails.unwrap_or(false);
    let ingest_data = input.ingest_data.unwrap_or(false);
    let runtime = super::support::runtime()?;
    Ok(runtime.block_on(async {
        if fire_and_forget {
            guardrail
                .fire_and_forget_request(guardrails, ingest_data, payload)
                .await;
            None
        } else {
            Some(
                guardrail
                    .send_request(guardrails, ingest_data, payload)
                    .await,
            )
        }
    }))
}

fn report(
    guardrail: &AktoMirroring,
    calls: &[ProviderCall],
    answer: Option<Result<AktoAnswer, AktoError>>,
) -> crate::helpers::HelperResult {
    let request = calls
        .first()
        .ok_or_else(|| failure("MockError", None, "no request reached the mock".to_owned()))
        .and_then(|call| captured_request(&call.request))?;
    match answer {
        None => Ok(json!({"request": request, "succeeded": true})),
        Some(Ok(answer)) => {
            let verdict = guardrail
                .handle_guardrail_response(&answer)
                .map_err(|error| failure("AktoError", None, error.to_string()))?;
            Ok(json!({"request": request, "allowed": verdict.allowed, "reason": verdict.reason}))
        }
        Some(Err(error)) => Err(failure("AktoError", None, error.to_string())),
    }
}

fn script(answer: &super::AnswerInput) -> Result<MockScript, Box<HelperError>> {
    let reply = UpstreamAnswer {
        status: answer.status.unwrap_or(200),
        headers: BTreeMap::new(),
        body: Some(answer.body.clone().unwrap_or(Value::Null)),
        events: None,
    };
    MockScript::try_from(std::slice::from_ref(&reply))
        .map_err(|error| failure("ScriptError", None, error.to_string()))
}

fn captured_request(request: &ProviderRequest) -> Result<Value, Box<HelperError>> {
    let (path, query) = match request.path.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (request.path.as_str(), None),
    };
    let mut params = serde_json::Map::new();
    if let Some(query) = query {
        for (name, value) in url::form_urlencoded::parse(query.as_bytes()) {
            params.insert(name.into_owned(), Value::String(value.into_owned()));
        }
    }
    let body = super::payload::normalize(&request.body)?;
    Ok(json!({
        "path": path,
        "query": Value::Object(params),
        "content_type": request.headers.get("content-type"),
        "body": body,
    }))
}
