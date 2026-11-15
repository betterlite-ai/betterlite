use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::hiddenlayer::evaluation::metadata;
use betterlite_guardrails::hiddenlayer::hiddenlayer_authentication::token::{
    JwtError, JwtTransport, TokenRequest,
};
use betterlite_guardrails::hiddenlayer::hiddenlayer_version_dispatch::{
    HiddenlayerGuardrailInstance, initialize_guardrail,
};
use betterlite_http::proxy::base_builder;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::case::UpstreamAnswer;
use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};
use crate::mock::MockProvider;
use crate::routing_mock_scripts::{MockScript, ProviderCall, redact};

const SAAS_HOST: &str = "api.hiddenlayer.ai";
const FIXED_ID: [u8; 16] = [0x11; 16];

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    params: Map<String, Value>,
    guardrail_name: String,
    saas: bool,
    tokens: Vec<String>,
    answers: Vec<UpstreamAnswer>,
    inputs: GuardrailApiInputs,
    request_data: BTreeMap<String, GuardrailValue>,
    input_type: Option<GuardrailInputType>,
}

#[derive(Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    #[default]
    Apply,
    Dispatch,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function {
            Function::Apply => apply(input),
            Function::Dispatch => dispatch(input),
        }
    }
}

fn apply(input: &Input) -> HelperResult {
    let script = MockScript::try_from(input.answers.as_slice()).or_value_error()?;
    let mock = MockProvider::start(script).or_value_error()?;
    let base = mock.base_url.clone();
    let address: SocketAddr = base
        .trim_start_matches("http://")
        .parse()
        .or_value_error()?;
    let api_base = if input.saas {
        format!("http://{SAAS_HOST}:{}", address.port())
    } else {
        base.clone()
    };
    let params = params(input, &api_base, &base)?;
    let builder = base_builder();
    let builder = if input.saas {
        builder.resolve(SAAS_HOST, address)
    } else {
        builder
    };
    let client = builder.build().or_config_error()?;
    let mut request_data = input.request_data.clone();
    let mut logging = logging_target(&request_data);
    let mut guardrail = instance(&params, input, client)?;
    if let HiddenlayerGuardrailInstance::V2(v2) = &mut guardrail {
        v2.new_id = || Uuid::from_bytes(FIXED_ID);
    }
    let input_type = input.input_type.unwrap_or(GuardrailInputType::Request);
    let evaluated = runtime()?.block_on(guardrail.apply_guardrail(
        input.inputs.clone(),
        &mut request_data,
        input_type,
        logging.as_mut(),
    ));
    let calls = mock.finish().or_value_error()?;
    let (raised, result) = match evaluated {
        Ok(evaluated) => {
            let messages = match evaluated.structured_messages.as_ref() {
                Some(messages) => serde_json::to_value(messages).or_value_error()?,
                None => Value::Null,
            };
            let result = json!({
                "texts": evaluated.texts,
                "structured_messages": messages,
                "tool_calls": evaluated.tool_calls,
            });
            (Value::Null, result)
        }
        Err(GuardrailFailure::HttpObject { status, detail }) => (
            json!({"kind": "http", "status": status, "detail": detail}),
            Value::Null,
        ),
        Err(_error) => (json!({"kind": "failed"}), Value::Null),
    };
    Ok(json!({
        "requests": described(&calls, &input.tokens),
        "raised": raised,
        "result": result,
        "roundtrip": roundtrip(&request_data),
    }))
}

fn dispatch(input: &Input) -> HelperResult {
    let params = params(input, "http://127.0.0.1:1", "http://127.0.0.1:1")?;
    let client = base_builder().build().or_config_error()?;
    let guardrail = instance(&params, input, client)?;
    Ok(json!({
        "supported_event_hooks": serde_json::to_value(guardrail.get_supported_event_hooks())
            .unwrap_or(Value::Null),
        "raised": Value::Null,
    }))
}

fn instance(
    params: &LitellmParams,
    input: &Input,
    client: Client,
) -> Result<HiddenlayerGuardrailInstance, Box<HelperError>> {
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let transport = Arc::new(ScriptedTokens::new(&input.tokens));
    initialize_guardrail(
        params,
        Some(&input.guardrail_name),
        &environment,
        client,
        transport,
    )
    .or_config_error()
}

// the auth layer hands out the case's tokens without an HTTP round trip, which the oracle compares separately
struct ScriptedTokens(Mutex<VecDeque<String>>);

impl ScriptedTokens {
    fn new(tokens: &[String]) -> Self {
        Self(Mutex::new(tokens.iter().cloned().collect()))
    }
}

impl JwtTransport for ScriptedTokens {
    fn fetch_token(&self, _request: &TokenRequest) -> Result<String, JwtError> {
        Ok(self.0.lock().map_or_else(
            |_poisoned| String::new(),
            |mut tokens| tokens.pop_front().unwrap_or_default(),
        ))
    }
}

fn params(
    input: &Input,
    api_base: &str,
    auth_url: &str,
) -> Result<LitellmParams, Box<HelperError>> {
    let mut fields = input.params.clone();
    fields.insert("api_base".to_owned(), Value::String(api_base.to_owned()));
    fields.insert("auth_url".to_owned(), Value::String(auth_url.to_owned()));
    serde_json::from_value(Value::Object(fields)).or_value_error()
}

fn logging_target(request_data: &BTreeMap<String, GuardrailValue>) -> Option<GuardrailLogTarget> {
    let Some(GuardrailValue::Object(logging)) = request_data.get("litellm_logging_obj") else {
        return None;
    };
    let Some(GuardrailValue::Object(details)) = logging.get("model_call_details") else {
        return None;
    };
    let model_call_params = match details.get("litellm_params") {
        Some(GuardrailValue::Object(params)) => Some(params.clone()),
        _ => None,
    };
    Some(GuardrailLogTarget {
        litellm_params: None,
        model_call_params,
    })
}

fn described(calls: &[ProviderCall], tokens: &[String]) -> Value {
    let requests: Vec<Value> = calls
        .iter()
        .map(|call| {
            let headers: BTreeMap<String, String> = call
                .request
                .headers
                .iter()
                .filter(|(name, _value)| name.starts_with("hl-") || name.as_str() == "content-type")
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect();
            json!({
                "path": call.request.path,
                "headers": headers,
                "authorization": bearer(&call.request.headers, tokens),
                "body": call.request.body,
            })
        })
        .collect();
    json!(requests)
}

// the mock redacts credentials, so the captured value is matched against the digest of each issued token
fn bearer(headers: &BTreeMap<String, String>, tokens: &[String]) -> Value {
    let Some(value) = headers.get("authorization") else {
        return Value::Null;
    };
    for (index, token) in tokens.iter().enumerate() {
        let mut probe = BTreeMap::from([("authorization".to_owned(), format!("Bearer {token}"))]);
        redact(&mut probe);
        if probe.get("authorization") == Some(value) {
            return json!(format!("bearer:{index}"));
        }
    }
    json!(value)
}

fn roundtrip(request_data: &BTreeMap<String, GuardrailValue>) -> Value {
    let Some(GuardrailValue::Object(proxy)) = request_data.get("proxy_server_request") else {
        return Value::Null;
    };
    let Some(GuardrailValue::Object(headers)) = proxy.get("headers") else {
        return Value::Null;
    };
    headers
        .get(metadata::ROUNDTRIP_HEADER)
        .and_then(|value| serde_json::to_value(value).ok())
        .unwrap_or(Value::Null)
}

fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .or_config_error()
}
