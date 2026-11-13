use std::collections::BTreeMap;
use std::net::TcpListener;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::{
    GenericGuardrailAPISettings, GuardrailInputType,
};
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::qohash::{QostodianNexus, QostodianNexusParams};
use serde_json::json;

use crate::case::UpstreamAnswer;
use crate::helpers::{HelperError, HelperResult};
use crate::mock::MockProvider;
use crate::routing_mock_scripts::{MockScript, ProviderCall};

pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QohashCall {
    settings: Settings,
    #[serde(default)]
    inputs: GuardrailApiInputs,
    #[serde(default)]
    request_data: BTreeMap<String, GuardrailValue>,
    input_type: GuardrailInputType,
    litellm_call_id: Option<String>,
    litellm_trace_id: Option<String>,
    answer: Option<UpstreamAnswer>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    guardrail_name: String,
    extra_headers: Vec<String>,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct CapturedRequest {
    input_type: GuardrailInputType,
    litellm_call_id: Option<String>,
    litellm_trace_id: Option<String>,
    texts: Option<Vec<String>>,
    images: Option<Vec<String>>,
    request_headers: Option<BTreeMap<String, String>>,
}

impl crate::helpers::Helper for Op {
    type Input = QohashCall;

    fn run(input: &Self::Input) -> HelperResult {
        call(input)
    }
}

fn call(input: &QohashCall) -> HelperResult {
    let (base, mock) = mock_base(input.answer.as_ref())?;
    let lookup = BTreeMap::<String, String>::new();
    let nexus = QostodianNexus::new(
        QostodianNexusParams {
            api_key: None,
            api_base: Some(base),
            guardrail_name: Some(input.settings.guardrail_name.clone()),
            extra_headers: Some(input.settings.extra_headers.clone()),
            provider_params: BTreeMap::new(),
            settings: GenericGuardrailAPISettings::from_optional(None),
            client: reqwest::Client::default(),
            timeout: Some(0.5_f64),
        },
        &lookup,
    )
    .map_err(|error| failure("ValueError", None, error.to_string()))?;
    let mut request_data = input.request_data.clone();
    let mut log_target = GuardrailLogTarget {
        litellm_params: Some(log_params(input)),
        model_call_params: None,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("OSError", None, error.to_string()))?;
    let result = runtime.block_on(nexus.apply_guardrail(
        input.inputs.clone(),
        &mut request_data,
        input.input_type,
        Some(&mut log_target),
    ));
    let recorded = finish(mock)?;
    let path = recorded
        .as_ref()
        .map(|provider_call| provider_call.request.path.clone());
    let content_type = recorded
        .as_ref()
        .and_then(|provider_call| provider_call.request.headers.get("content-type").cloned());
    let request: Option<CapturedRequest> = recorded
        .as_ref()
        .and_then(|provider_call| serde_json::from_value(provider_call.request.body.clone()).ok());
    match result {
        Ok(inputs) => Ok(json!({
            "guardrail_name": nexus.guardrail_name,
            "path": path,
            "content_type": content_type,
            "request": request,
            "inputs": inputs,
        })),
        Err(error) => Err(helper_failure(error)),
    }
}

fn log_params(input: &QohashCall) -> BTreeMap<String, GuardrailValue> {
    let mut params = BTreeMap::new();
    if let Some(call_id) = &input.litellm_call_id {
        params.insert(
            "litellm_call_id".to_owned(),
            GuardrailValue::Text(call_id.clone()),
        );
    }
    if let Some(trace_id) = &input.litellm_trace_id {
        params.insert(
            "litellm_trace_id".to_owned(),
            GuardrailValue::Text(trace_id.clone()),
        );
    }
    params
}

fn mock_base(
    answer: Option<&UpstreamAnswer>,
) -> Result<(String, Option<MockProvider>), Box<HelperError>> {
    let Some(reply) = answer else {
        let silent = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|error| failure("OSError", None, error.to_string()))?;
        let base = format!(
            "http://{}",
            silent
                .local_addr()
                .map_err(|error| failure("OSError", None, error.to_string()))?
        );
        return Ok((base, None));
    };
    let script = MockScript::try_from(std::slice::from_ref(reply))
        .map_err(|error| failure("MockError", None, error.to_string()))?;
    let provider = MockProvider::start(script)
        .map_err(|error| failure("MockError", None, error.to_string()))?;
    let base = provider.base_url.clone();
    Ok((base, Some(provider)))
}

fn finish(mock: Option<MockProvider>) -> Result<Option<ProviderCall>, Box<HelperError>> {
    let Some(provider) = mock else {
        return Ok(None);
    };
    let mut calls = provider
        .finish()
        .map_err(|error| failure("MockError", None, error.to_string()))?;
    Ok(calls.pop())
}

fn helper_failure(error: GuardrailFailure) -> Box<HelperError> {
    let message = error.to_string();
    match error {
        GuardrailFailure::GuardrailRaised { status_code, .. } => {
            failure("GuardrailRaisedException", Some(status_code), message)
        }
        GuardrailFailure::ModifyResponse(_)
        | GuardrailFailure::BlockedPii { .. }
        | GuardrailFailure::SensitiveDataRoute(_)
        | GuardrailFailure::Http { .. }
        | GuardrailFailure::HttpObject { .. }
        | GuardrailFailure::Other(_) => failure("Exception", None, message),
    }
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
