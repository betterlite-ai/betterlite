use std::collections::BTreeMap;
use std::net::TcpListener;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::singulr::SingulrGuardrail;
use betterlite_guardrails::singulr::error::STATUS_CODE;
use betterlite_guardrails::singulr::error::SingulrError;
use betterlite_guardrails::singulr::init::SingulrArgs;
use betterlite_http::proxy::base_builder;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::case::UpstreamAnswer;
use crate::helpers::{HelperError, HelperResult};
use crate::mock::MockProvider;
use crate::routing_mock_scripts::{MockScript, ProviderCall};

const REPORTED_HEADERS: [&str; 4] = [
    "content-type",
    "x-singulr-gateway-token",
    "x-singulr-enforcement-entity-id",
    "x-singulr-guardrail-id",
];

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SingulrCall {
    scenarios: Vec<Scenario>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    name: String,
    settings: Settings,
    #[serde(default)]
    inputs: GuardrailApiInputs,
    #[serde(default)]
    request_data: BTreeMap<String, GuardrailValue>,
    input_type: GuardrailInputType,
    #[serde(default)]
    answer: Option<UpstreamAnswer>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    api_key: Option<String>,
    api_base: Option<String>,
    application_id: Option<String>,
    guardrail_id: Option<String>,
    block_on_error: Option<bool>,
    timeout: Option<f64>,
}

impl crate::helpers::Helper for Op {
    type Input = SingulrCall;

    fn run(input: &Self::Input) -> HelperResult {
        let mut scenarios = Vec::with_capacity(input.scenarios.len());
        for scenario in &input.scenarios {
            scenarios.push(scenario_value(scenario)?);
        }
        Ok(json!({
            "supported_event_hooks": betterlite_guardrails::singulr::SUPPORTED_EVENT_HOOKS,
            "scenarios": scenarios,
        }))
    }
}

fn scenario_value(scenario: &Scenario) -> Result<Value, Box<HelperError>> {
    let (mock, base) = start_mock(scenario.answer.as_ref())?;
    let guardrail = build_guardrail(&scenario.settings, &base)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("OSError", error.to_string()))?;
    let result = runtime.block_on(guardrail.apply_guardrail(
        scenario.inputs.clone(),
        &scenario.request_data,
        scenario.input_type,
        None,
    ));
    let calls = finish_calls(mock)?;
    let failed_calls = if scenario.answer.is_some() {
        calls
    } else {
        None
    };
    Ok(match result {
        Ok(inputs) => json!({
            "name": scenario.name,
            "outcome": "allowed",
            "status": Value::Null,
            "calls": calls,
            "inputs": inputs,
        }),
        Err(error) => json!({
            "name": scenario.name,
            "outcome": if matches!(error, SingulrError::Blocked { .. }) { "blocked" } else { "error" },
            "status": STATUS_CODE,
            "calls": failed_calls,
            "inputs": Value::Null,
        }),
    })
}

fn start_mock(
    answer: Option<&UpstreamAnswer>,
) -> Result<(Option<MockProvider>, String), Box<HelperError>> {
    let Some(answer) = answer else {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|error| failure("OSError", error.to_string()))?;
        let address = listener
            .local_addr()
            .map_err(|error| failure("OSError", error.to_string()))?;
        drop(listener);
        return Ok((None, format!("http://{address}")));
    };
    let script = MockScript::try_from(std::slice::from_ref(answer))
        .map_err(|error| failure("MockError", error.to_string()))?;
    let provider =
        MockProvider::start(script).map_err(|error| failure("MockError", error.to_string()))?;
    let base = provider.base_url.clone();
    Ok((Some(provider), base))
}

fn build_guardrail(settings: &Settings, base: &str) -> Result<SingulrGuardrail, Box<HelperError>> {
    let args = SingulrArgs {
        name: "singulr".to_owned(),
        singulr_api_key: settings.api_key.clone().map(SecretString::from),
        singulr_api_base: settings.api_base.clone(),
        singulr_application_id: settings.application_id.clone(),
        singulr_guardrail_id: settings.guardrail_id.clone(),
        block_on_error: settings.block_on_error,
        timeout: settings.timeout,
        event_hook: LitellmParamsMode::One("pre_call".to_owned()),
        default_on: false,
    };
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let client = base_builder()
        .build()
        .map_err(|error| failure("OSError", error.to_string()))?;
    let mut guardrail = SingulrGuardrail::new(&args, &environment, client)
        .map_err(|error| failure("ValueError", error.to_string()))?;
    base.clone_into(&mut guardrail.settings.api_base);
    Ok(guardrail)
}

fn finish_calls(mock: Option<MockProvider>) -> Result<Option<Vec<Value>>, Box<HelperError>> {
    let Some(mock) = mock else {
        return Ok(None);
    };
    let calls = mock
        .finish()
        .map_err(|error| failure("MockError", error.to_string()))?;
    Ok(Some(calls.iter().map(call_value).collect()))
}

fn call_value(call: &ProviderCall) -> Value {
    let mut headers = BTreeMap::new();
    for name in REPORTED_HEADERS {
        headers.insert(name.to_owned(), call.request.headers.get(name).cloned());
    }
    json!({
        "path": call.request.path,
        "headers": headers,
        "body": call.request.body,
    })
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
