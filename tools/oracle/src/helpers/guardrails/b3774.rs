use std::collections::BTreeMap;
use std::net::TcpListener;
use std::time::{Duration, SystemTime};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract as _;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::headroom::headroom_compression::{HeadroomGuardrail, HeadroomRuntime};
use betterlite_guardrails::headroom_settings::{
    HeadroomSettings, UnreachableFallback, resolve_timeout,
};
use betterlite_http::proxy::base_builder;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::case::UpstreamAnswer;
use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};
use crate::mock::MockProvider;
use crate::routing_mock_scripts::{MockScript, ProviderCall};

const CALL_ID: &str = "litellm_call_id";
const UNREACHABLE_BASE: &str = "http://127.0.0.1:1";
const TRANSPORT_DETAIL: &str = "<transport-error>";

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct CompressionCall {
    api_key: Option<String>,
    model: Option<String>,
    unreachable_fallback: Option<UnreachableFallback>,
    timeout: Option<f64>,
    inputs: GuardrailApiInputs,
    request_data: BTreeMap<String, GuardrailValue>,
    input_type: Option<GuardrailInputType>,
    logging_call_id: Option<String>,
    answer: Option<UpstreamAnswer>,
    silent: bool,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = CompressionCall;

    fn run(input: &Self::Input) -> HelperResult {
        let (mock, _silent, base) = start_service(input)?;
        let guardrail = build_guardrail(input, &base)?;
        let mut request_data = input.request_data.clone();
        let input_type = input.input_type.unwrap_or(GuardrailInputType::Request);
        let mut target = logging_target(input.logging_call_id.as_deref());
        let (inputs, raised) = call(
            &guardrail,
            input,
            input_type,
            &mut request_data,
            target.as_mut(),
        )?;
        let calls = mock
            .map(MockProvider::finish)
            .transpose()
            .or_config_error()?
            .unwrap_or_default();
        let requests: Vec<Value> = calls.iter().map(describe).collect();
        let (call_id, generated) = resolve_identity(input, &request_data);
        let call_id_is_uuid = generated
            && call_id
                .as_deref()
                .is_some_and(|id| Uuid::parse_str(id).is_ok());
        let issued = call_id
            .as_deref()
            .and_then(|call_id| guardrail.has_issued_hashes(call_id))
            .map(|hashes| hashes.into_iter().collect::<Vec<_>>());
        let request_data = serde_json::to_value(&request_data).or_value_error()?;
        Ok(json!({
            "inputs": inputs,
            "raised": raised,
            "request_data": request_data,
            "calls": requests,
            "call_id_is_uuid": call_id_is_uuid,
            "issued": issued,
        }))
    }
}

fn call(
    guardrail: &HeadroomGuardrail,
    input: &CompressionCall,
    input_type: GuardrailInputType,
    request_data: &mut BTreeMap<String, GuardrailValue>,
    target: Option<&mut GuardrailLogTarget>,
) -> Result<(Option<GuardrailApiInputs>, Option<Value>), Box<HelperError>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .or_config_error()?;
    let result = runtime.block_on(guardrail.apply_guardrail(
        input.inputs.clone(),
        request_data,
        input_type,
        target,
    ));
    match result {
        Ok(returned) => Ok((Some(returned), None)),
        Err(GuardrailFailure::HttpObject { status, mut detail }) => {
            // httpx and reqwest word the same transport failure differently, so both sides report one marker
            if let Some(transport) = detail.get_mut("detail") {
                *transport = GuardrailValue::Text(TRANSPORT_DETAIL.to_owned());
            }
            let encoded = serde_json::to_value(detail).or_value_error()?;
            Ok((None, Some(json!({"status": status, "detail": encoded}))))
        }
        Err(other) => Err(other).or_config_error(),
    }
}

fn resolve_identity(
    input: &CompressionCall,
    request_data: &BTreeMap<String, GuardrailValue>,
) -> (Option<String>, bool) {
    let logging = input.logging_call_id.as_deref().filter(|id| !id.is_empty());
    if let Some(call_id) = logging.or_else(|| text_field(&input.request_data, CALL_ID)) {
        return (Some(call_id.to_owned()), false);
    }
    let after = text_field(request_data, CALL_ID);
    (after.map(str::to_owned), after.is_some())
}

fn text_field<'data>(
    data: &'data BTreeMap<String, GuardrailValue>,
    key: &str,
) -> Option<&'data str> {
    match data.get(key) {
        Some(GuardrailValue::Text(text)) if !text.is_empty() => Some(text.as_str()),
        _ => None,
    }
}

fn logging_target(call_id: Option<&str>) -> Option<GuardrailLogTarget> {
    call_id.map(|call_id| GuardrailLogTarget {
        litellm_params: None,
        model_call_params: Some(BTreeMap::from([(
            CALL_ID.to_owned(),
            GuardrailValue::Text(call_id.to_owned()),
        )])),
    })
}

fn build_guardrail(
    input: &CompressionCall,
    base: &str,
) -> Result<HeadroomGuardrail, Box<HelperError>> {
    let settings = HeadroomSettings {
        api_base: base.trim_end_matches('/').to_owned(),
        api_key: input.api_key.clone().map(SecretString::from),
        model: input.model.clone(),
        unreachable_fallback: input.unreachable_fallback.unwrap_or_default(),
        timeout: resolve_timeout(input.timeout),
        ccr_retrieval: true,
    };
    HeadroomGuardrail::new(
        Some("headroom".to_owned()),
        settings,
        Box::new(|connect: Duration| base_builder().connect_timeout(connect).build()),
        HeadroomRuntime {
            clock: Box::new(|| Duration::ZERO),
            wall: SystemTime::now,
            new_id: Uuid::new_v4,
            event_hook: None,
        },
    )
    .or_config_error()
}

fn start_service(
    input: &CompressionCall,
) -> Result<(Option<MockProvider>, Option<TcpListener>, String), Box<HelperError>> {
    if input.silent {
        let listener = TcpListener::bind(("127.0.0.1", 0)).or_config_error()?;
        let base = format!("http://{}", listener.local_addr().or_config_error()?);
        return Ok((None, Some(listener), base));
    }
    let Some(answer) = input.answer.as_ref() else {
        return Ok((None, None, UNREACHABLE_BASE.to_owned()));
    };
    let mock = MockScript::try_from(std::slice::from_ref(answer))
        .and_then(MockProvider::start)
        .or_config_error()?;
    let base = mock.base_url.clone();
    Ok((Some(mock), None, base))
}

fn describe(call: &ProviderCall) -> Value {
    let request = &call.request;
    json!({
        "method": request.method,
        "path": request.path,
        "authorization_present": request.headers.contains_key("authorization"),
        "content_type": request.headers.get("content-type"),
        "body": request.body,
    })
}
