use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::zscaler_ai_guard::error::ZscalerAiGuardError;
use betterlite_guardrails::zscaler_ai_guard::init::ZscalerArgs;
use betterlite_guardrails::zscaler_ai_guard::{Inputs, SUPPORTED_EVENT_HOOKS, ZscalerAiGuard};
use betterlite_http::proxy::base_builder;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;

use crate::case::UpstreamAnswer;
use crate::helpers::{HelperError, HelperResult};
use crate::mock::MockProvider;
use crate::routing_mock_scripts::MockScript;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ZscalerCall {
    settings: Settings,
    inputs: Inputs,
    request_data: BTreeMap<String, GuardrailValue>,
    input_type: GuardrailInputType,
    answer: UpstreamAnswer,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    api_base: Option<String>,
    api_key: Option<String>,
    policy_id: Option<i64>,
    send_user_api_key_alias: Option<bool>,
    send_user_api_key_user_id: Option<bool>,
    send_user_api_key_team_id: Option<bool>,
    timeout: Option<f64>,
}

impl crate::helpers::Helper for Op {
    type Input = ZscalerCall;

    fn run(input: &Self::Input) -> HelperResult {
        call(input)
    }
}

fn call(input: &ZscalerCall) -> HelperResult {
    let mock = start_mock(&input.answer)?;
    let guardrail = build_guardrail(input, &mock.base_url)?;
    let runtime = runtime()?;
    let result = runtime.block_on(guardrail.apply_guardrail(
        input.inputs.clone(),
        &input.request_data,
        input.input_type,
    ));
    let recorded = mock
        .finish()
        .map_err(|error| failure("MockError", None, error.to_string()))?
        .into_iter()
        .next();
    output(result, recorded)
}

fn start_mock(answer: &UpstreamAnswer) -> Result<MockProvider, Box<HelperError>> {
    let script = MockScript::try_from(std::slice::from_ref(answer))
        .map_err(|error| failure("MockError", None, error.to_string()))?;
    MockProvider::start(script).map_err(|error| failure("MockError", None, error.to_string()))
}

fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("OSError", None, error.to_string()))
}

fn output(
    result: Result<Inputs, ZscalerAiGuardError>,
    recorded: Option<crate::routing_mock_scripts::ProviderCall>,
) -> HelperResult {
    let (path, headers, body) = match recorded {
        Some(provider_call) => (
            Some(provider_call.request.path),
            Some(provider_call.request.headers),
            Some(provider_call.request.body),
        ),
        None => (None, None, None),
    };
    match result {
        Ok(inputs) => Ok(json!({
            "supported_event_hooks": SUPPORTED_EVENT_HOOKS,
            "path": path,
            "headers": header_subset(headers.as_ref()),
            "body": body,
            "inputs": inputs,
        })),
        Err(error @ ZscalerAiGuardError::Blocked { .. }) => Ok(json!({
            "blocked": true,
            "status": error.status_code().as_u16(),
            "detail": error.detail(),
            "path": path,
            "headers": header_subset(headers.as_ref()),
            "body": body,
        })),
        Err(error) => Err(failure(
            "HTTPException",
            Some(error.status_code().as_u16()),
            error.to_string(),
        )),
    }
}

fn build_guardrail(input: &ZscalerCall, base: &str) -> Result<ZscalerAiGuard, Box<HelperError>> {
    let api_base = match input.settings.api_base.as_deref() {
        Some(configured) => {
            let parsed = url::Url::parse(configured)
                .map_err(|error| failure("ValueError", None, error.to_string()))?;
            format!("{base}{}", parsed.path())
        }
        None => base.to_owned(),
    };
    let args = ZscalerArgs {
        name: "zscaler-ai-guard".to_owned(),
        api_base: Some(api_base),
        api_key: input.settings.api_key.clone().map(SecretString::from),
        policy_id: input.settings.policy_id,
        send_user_api_key_alias: input.settings.send_user_api_key_alias,
        send_user_api_key_user_id: input.settings.send_user_api_key_user_id,
        send_user_api_key_team_id: input.settings.send_user_api_key_team_id,
        timeout: input.settings.timeout,
        event_hook: LitellmParamsMode::One("pre_call".to_owned()),
        default_on: false,
    };
    let client = base_builder()
        .build()
        .map_err(|error| failure("OSError", None, error.to_string()))?;
    ZscalerAiGuard::new(args, &BTreeMap::new(), client)
        .map_err(|error| failure("ValueError", None, error.to_string()))
}

fn header_subset(headers: Option<&BTreeMap<String, String>>) -> Option<BTreeMap<String, String>> {
    const NAMES: [&str; 5] = [
        "authorization",
        "content-type",
        "user-api-key-alias",
        "user-api-key-team-id",
        "user-api-key-user-id",
    ];
    headers.map(|headers| {
        NAMES
            .iter()
            .filter_map(|name| {
                headers
                    .get(*name)
                    .map(|value| ((*name).to_owned(), value.clone()))
            })
            .collect()
    })
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
