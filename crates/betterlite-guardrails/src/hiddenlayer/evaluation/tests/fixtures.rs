use std::collections::BTreeMap;
use std::net::SocketAddr;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_http::proxy::base_builder;
use reqwest::Client;
use secrecy::SecretString;

use crate::generic_guardrail_contracts::GuardrailMessage;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::hiddenlayer::evaluation::EvaluationArgs;
use crate::primitives::json_value::JsonValue;

pub(crate) const SAAS_HOST: &str = "api.hiddenlayer.ai";

pub(crate) fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

pub(crate) fn object(entries: Vec<(&str, GuardrailValue)>) -> GuardrailValue {
    GuardrailValue::Object(
        entries
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    )
}

pub(crate) fn headers_value(headers: &[(&str, &str)]) -> GuardrailValue {
    object(
        headers
            .iter()
            .map(|(name, value)| (*name, text(value)))
            .collect(),
    )
}

pub(crate) fn proxy_data(headers: &[(&str, &str)]) -> BTreeMap<String, GuardrailValue> {
    BTreeMap::from([(
        "proxy_server_request".to_owned(),
        object(vec![("headers", headers_value(headers))]),
    )])
}

pub(crate) fn logging_data(model: &str) -> BTreeMap<String, GuardrailValue> {
    BTreeMap::from([(
        "litellm_logging_obj".to_owned(),
        object(vec![("model", text(model))]),
    )])
}

pub(crate) fn message(role: &str, content: GuardrailValue) -> GuardrailMessage {
    GuardrailMessage {
        role: role.to_owned(),
        extra: BTreeMap::from([("content".to_owned(), content)]),
    }
}

pub(crate) fn text_inputs(texts: &[&str]) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: Some(texts.iter().map(|value| (*value).to_owned()).collect()),
        ..GuardrailApiInputs::default()
    }
}

pub(crate) fn canonical(body: &str) -> String {
    let parsed: JsonValue = serde_json::from_str(body).expect("a JSON body");
    serde_json::to_string(&parsed).expect("JSON text")
}

pub(crate) fn args(api_base: &str) -> EvaluationArgs {
    EvaluationArgs {
        guardrail_name: Some("hiddenlayer-test".to_owned()),
        event_hook: LitellmParamsMode::One("pre_call".to_owned()),
        api_id: Some("api-id".to_owned()),
        api_key: Some(SecretString::from("api-key".to_owned())),
        api_base: Some(api_base.to_owned()),
        auth_url: Some("http://auth.invalid".to_owned()),
        timeout: Some(5.0),
        default_on: false,
    }
}

pub(crate) fn environment() -> BTreeMap<String, String> {
    BTreeMap::new()
}

pub(crate) fn client() -> Client {
    base_builder().build().expect("a client")
}

// the SaaS hostname earns the JWT, while the resolve steers it at the local server
pub(crate) fn saas_client(port: u16) -> Client {
    base_builder()
        .resolve(SAAS_HOST, SocketAddr::from(([127, 0, 0, 1], port)))
        .build()
        .expect("a client")
}

pub(crate) fn saas_base(port: u16) -> String {
    format!("http://{SAAS_HOST}:{port}")
}
