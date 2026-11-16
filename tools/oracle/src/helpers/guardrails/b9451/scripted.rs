use std::collections::BTreeMap;
use std::fmt::Display;
use std::net::TcpListener;

use betterlite_guardrails::aim::error::AimError;
use betterlite_guardrails::akto::error::AktoError;
use betterlite_guardrails::lasso_masking::wire::ModelResponseShape;
use betterlite_guardrails::third_party_hooks_a::error::ThirdPartyHookError;
use serde::Deserialize;
use serde_json::{Value, json};
use url::form_urlencoded;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_runner::case::UpstreamAnswer;
use betterlite_oracle_runner::mock::{MockProvider, ProviderRequest};
use betterlite_oracle_runner::routing_mock_scripts::{MockScript, redact_credentials};

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Hook {
    PreCall,
    Moderation,
    PostCall,
}

pub(super) fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: Display,
{
    Box::new(HelperError {
        class: "OracleHelperError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

pub(super) struct Scripted {
    pub(super) base_url: String,
    mock: Option<MockProvider>,
    _silent: TcpListener,
}

impl Scripted {
    pub(super) fn start(answers: &[UpstreamAnswer]) -> Result<Self, Box<HelperError>> {
        let silent = TcpListener::bind(("127.0.0.1", 0)).map_err(failure)?;
        let mock = if answers.is_empty() {
            None
        } else {
            let script = MockScript::try_from(answers).map_err(failure)?;
            Some(MockProvider::start(script).map_err(failure)?)
        };
        let base_url = match &mock {
            Some(provider) => provider.base_url.clone(),
            None => format!("http://{}", silent.local_addr().map_err(failure)?),
        };
        Ok(Self {
            base_url,
            mock,
            _silent: silent,
        })
    }

    pub(super) fn take_requests(self) -> Result<Vec<ProviderRequest>, Box<HelperError>> {
        let calls = self
            .mock
            .map(MockProvider::finish)
            .transpose()
            .map_err(failure)?;
        Ok(calls
            .unwrap_or_default()
            .into_iter()
            .map(|call| call.request)
            .collect())
    }
}

fn error_view(error: &ThirdPartyHookError) -> Value {
    let status = error.status_code().as_u16();
    match error {
        ThirdPartyHookError::Rejection {
            detail,
            openai_code,
            ..
        } => json!({"status": status, "detail": detail, "code": openai_code}),
        ThirdPartyHookError::Aim(AimError::Rejected { message, code }) => {
            json!({"status": status, "detail": message, "code": code})
        }
        ThirdPartyHookError::Akto(AktoError::Unreachable) => {
            json!({"status": status, "detail": "Akto guardrail service unreachable", "code": null})
        }
        ThirdPartyHookError::Failure { .. }
        | ThirdPartyHookError::ConductMissingPackage(_)
        | ThirdPartyHookError::Aim(_)
        | ThirdPartyHookError::Akto(_)
        | ThirdPartyHookError::Lakera(_)
        | ThirdPartyHookError::Lasso(_) => {
            json!({"status": status, "detail": error.to_string(), "code": null})
        }
    }
}

pub(super) fn outcome_view(
    requests: &[ProviderRequest],
    names: &[&str],
    secrets: &[(&str, String)],
    outcome: &Result<Value, ThirdPartyHookError>,
) -> Value {
    let seen: Vec<Value> = requests
        .iter()
        .map(|request| {
            let (path, raw_query) = request
                .path
                .split_once('?')
                .unwrap_or((request.path.as_str(), ""));
            let query: BTreeMap<String, String> = form_urlencoded::parse(raw_query.as_bytes())
                .into_owned()
                .collect();
            let shown: BTreeMap<&str, &String> = names
                .iter()
                .filter_map(|name| request.headers.get(*name).map(|value| (*name, value)))
                .collect();
            let checked: BTreeMap<&str, bool> = secrets
                .iter()
                .map(|(name, expected)| {
                    let mut hashed = BTreeMap::from([((*name).to_owned(), expected.clone())]);
                    redact_credentials(&mut hashed);
                    (*name, request.headers.get(*name) == hashed.get(*name))
                })
                .collect();
            json!({"path": path, "query": query, "headers": shown, "secrets": checked, "body": request.body})
        })
        .collect();
    match outcome {
        Ok(result) => json!({"result": result, "error": null, "requests": seen}),
        Err(error) => json!({"result": null, "error": error_view(error), "requests": seen}),
    }
}

pub(super) fn chat_response(outputs: &[String]) -> Result<ModelResponseShape, Box<HelperError>> {
    let choices: Vec<Value> = outputs
        .iter()
        .map(|text| json!({"message": {"role": "assistant", "content": text}}))
        .collect();
    serde_json::from_str(&json!({"choices": choices}).to_string()).map_err(failure)
}

pub(super) fn contents(response: &ModelResponseShape) -> Value {
    let texts: Vec<Option<String>> = response
        .choices
        .iter()
        .map(|choice| choice.message.as_ref()?.content.clone())
        .collect();
    json!({"contents": texts})
}
