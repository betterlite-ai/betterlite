#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::vigil_runtime::Settings as EnvironmentDefaults;
use reqwest::StatusCode;
use secrecy::SecretString;

use crate::generic_guardrail_contracts::{
    GuardrailInputType, GuardrailMessage, GuardrailToolParam,
};
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::third_party_init_b::vigil_guard::VigilGuardArgs;
use crate::vigil_guard::handler_protocol::HttpResponse;

use super::cases::{Expected, ScanResponse, Scenario};

pub(crate) const HARMFUL: &str = "Harmful content";
pub(crate) const BACKEND_CLOSED: &str =
    "Vigil Guard backend unreachable; request blocked by fail_closed policy.";
pub(crate) const UNRECOGNIZED: &str = "Vigil Guard returned an unrecognized decision.";

pub(crate) fn args(
    api_base: Option<&str>,
    api_key: Option<&str>,
    unreachable_fallback: &str,
    timeout: Option<f64>,
) -> VigilGuardArgs {
    VigilGuardArgs {
        name: String::new(),
        api_base: api_base.map(str::to_owned),
        api_key: api_key.map(SecretString::from),
        unreachable_fallback: unreachable_fallback.to_owned(),
        timeout,
        event_hook: LitellmParamsMode::One("pre_call".to_owned()),
        default_on: false,
    }
}

pub(crate) fn env_defaults(api_base: &str, api_key: &str) -> EnvironmentDefaults {
    EnvironmentDefaults {
        api_base: Some(api_base.to_owned()),
        api_key: Some(SecretString::from(api_key)),
    }
}

pub(crate) fn loaded_defaults(source: &str, variables: &[(&str, &str)]) -> EnvironmentDefaults {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .expect("the fixture config loads")
        .sections
        .vigil_runtime
}

pub(crate) fn header_args(api_key: &str, fallback: &str) -> VigilGuardArgs {
    args(Some("https://vigil.example"), Some(api_key), fallback, None)
}

pub(crate) fn request(texts: Option<Vec<&str>>, responses: Vec<ScanResponse>) -> Scenario {
    Scenario {
        args: args(
            Some("https://vigil.example"),
            Some("sk-vigil-test"),
            "fail_closed",
            None,
        ),
        inputs: GuardrailApiInputs {
            texts: texts.map(|items| items.iter().map(|text| (*text).to_owned()).collect()),
            images: None,
            tools: None,
            tool_calls: None,
            structured_messages: None,
            model: None,
            stream_holdback_chars: None,
        },
        input_type: GuardrailInputType::Request,
        request_data: BTreeMap::new(),
        logging: None,
        responses,
        analyzed: Vec::new(),
        metadata: Vec::new(),
        expected: Expected::Unchanged,
    }
}

pub(crate) fn response(
    texts: Option<Vec<&str>>,
    calls: &[(&str, &str)],
    responses: Vec<ScanResponse>,
) -> Scenario {
    let mut scenario = request(texts, responses);
    scenario.input_type = GuardrailInputType::Response;
    let tool_calls = calls.iter().map(|(id, arguments)| tool_call(id, arguments));
    scenario.inputs.tool_calls = Some(tool_calls.collect());
    scenario
}

pub(crate) fn rewritten(texts: &[&str], tool_calls: Option<Vec<GuardrailValue>>) -> Expected {
    Expected::Rewritten {
        texts: texts.iter().map(|text| (*text).to_owned()).collect(),
        tool_calls,
    }
}

pub(crate) fn raised(message: &str, status_code: u16, blocked_content: bool) -> Expected {
    Expected::Raised {
        message: message.to_owned(),
        status_code,
        blocked_content,
    }
}

pub(crate) fn other(fragment: &str) -> Expected {
    Expected::Other(fragment.to_owned())
}

pub(crate) fn tool_call(id: &str, arguments: &str) -> GuardrailValue {
    let mut function = BTreeMap::new();
    function.insert("name".to_owned(), GuardrailValue::Text("lookup".to_owned()));
    function.insert(
        "arguments".to_owned(),
        GuardrailValue::Text(arguments.to_owned()),
    );
    let mut fields = BTreeMap::new();
    fields.insert("id".to_owned(), GuardrailValue::Text(id.to_owned()));
    fields.insert(
        "type".to_owned(),
        GuardrailValue::Text("function".to_owned()),
    );
    fields.insert("function".to_owned(), GuardrailValue::Object(function));
    GuardrailValue::Object(fields)
}

pub(crate) fn tool_param() -> GuardrailToolParam {
    let mut function = BTreeMap::new();
    function.insert("name".to_owned(), GuardrailValue::Text("lookup".to_owned()));
    let mut extra = BTreeMap::new();
    extra.insert("function".to_owned(), GuardrailValue::Object(function));
    GuardrailToolParam {
        tool_type: "function".to_owned(),
        extra,
    }
}

pub(crate) fn message() -> GuardrailMessage {
    let mut extra = BTreeMap::new();
    extra.insert(
        "content".to_owned(),
        GuardrailValue::Text("secret".to_owned()),
    );
    GuardrailMessage {
        role: "user".to_owned(),
        extra,
    }
}

pub(crate) fn allowed() -> ScanResponse {
    Ok(json_body(r#"{"decision": "ALLOWED"}"#))
}

pub(crate) fn sanitized(text: &str) -> ScanResponse {
    Ok(json_body(&format!(
        r#"{{"decision": "SANITIZED", "sanitizedText": "{text}"}}"#
    )))
}

pub(crate) fn blocked(reason: &str) -> ScanResponse {
    Ok(json_body(&format!(
        r#"{{"decision": "BLOCKED", "blockMessage": "{reason}"}}"#
    )))
}

pub(crate) fn missing_decision() -> ScanResponse {
    Ok(json_body("{}"))
}

pub(crate) fn non_object_answer() -> ScanResponse {
    Ok(json_body("null"))
}

pub(crate) fn unknown_decision() -> ScanResponse {
    Ok(json_body(r#"{"decision": "UNKNOWN"}"#))
}

pub(crate) fn unavailable() -> ScanResponse {
    Ok(HttpResponse {
        status: StatusCode::BAD_REQUEST,
        body: Vec::new(),
    })
}

fn json_body(body: &str) -> HttpResponse {
    HttpResponse {
        status: StatusCode::OK,
        body: body.as_bytes().to_vec(),
    }
}
