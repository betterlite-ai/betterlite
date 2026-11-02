use std::collections::BTreeMap;
use std::net::TcpListener;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::aporia_guardrail::error::AporiaGuardrailError;
use betterlite_guardrails::aporia_guardrail::{
    AporiaGuardrail, ModerationGate, Settings as AporiaSettings,
};
use betterlite_guardrails::generic_guardrail_contracts::GuardrailMessage;
use secrecy::SecretString;
use serde_json::json;

use super::{OracleBlockable, finish, prepare};
use crate::case::UpstreamAnswer;
use crate::helpers::{HelperError, HelperResult};
use crate::mock::MockProvider;

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

pub(super) enum AporiaCall {
    Moderation(Option<Vec<GuardrailMessage>>),
    PostCall(Vec<GuardrailMessage>, Option<String>),
}

pub(super) fn run_aporia(
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
    finish(mock, result, |_body, _ran| {})
}

impl OracleBlockable for AporiaGuardrailError {
    fn blocked_response(&self) -> Option<serde_json::Value> {
        match self {
            Self::Blocked { response } => Some(json!(response)),
            Self::MissingEnvironment { .. }
            | Self::Timeout { .. }
            | Self::Request { .. }
            | Self::InvalidHeader { .. }
            | Self::HttpStatus { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. } => None,
        }
    }

    fn status_u16(&self) -> u16 {
        self.status_code().as_u16()
    }

    fn classify(&self) -> &'static str {
        match self {
            Self::Blocked { .. } => "HTTPException",
            Self::MissingEnvironment { .. } => "KeyError",
            Self::Timeout { .. }
            | Self::Request { .. }
            | Self::InvalidHeader { .. }
            | Self::HttpStatus { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. } => "AporiaGuardrailAPIError",
        }
    }
}
