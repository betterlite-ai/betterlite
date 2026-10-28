use betterlite_http::openai_mock_transport::{
    DecodeMockRequestError, MockModel, TransportChoice, decode_mock_request,
    mock_chat_completion, select_transport,
};
use serde_json::value::RawValue;
use uuid::Uuid;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(tag = "case", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    #[serde(rename = "mock_reply_literals")]
    ReplyLiterals {
        model: String,
        clock: i64,
        uuid: String,
    },
    #[serde(rename = "mock_request_decoder")]
    RequestDecoder { probes: Vec<Probe> },
    #[serde(rename = "mock_selection")]
    Selection { scenarios: Vec<SelectionScenario> },
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    pub(crate) content: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SelectionScenario {
    pub(crate) kind: SelectionKind,
    pub(crate) network_mock: bool,
    pub(crate) supplied: SuppliedSession,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SelectionKind {
    Async,
    Sync,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SuppliedSession {
    None,
    Client,
    Shared,
}

fn invalid(message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn decode_probe(probe: &Probe) -> serde_json::Value {
    match decode_mock_request(probe.content.as_bytes()) {
        Ok(decoded) => serde_json::json!({"model": decoded.model, "stream": decoded.stream}),
        Err(error @ DecodeMockRequestError::NotAMapping { .. }) => {
            serde_json::json!({"error": error.to_string()})
        }
    }
}

fn selection_outcome(scenario: &SelectionScenario) -> &'static str {
    let client_session = matches!(scenario.supplied, SuppliedSession::Client).then_some(());
    let shared_session = matches!(scenario.supplied, SuppliedSession::Shared).then_some(());
    match select_transport(client_session, scenario.network_mock, shared_session) {
        TransportChoice::Supplied(()) => "supplied",
        TransportChoice::Mock => "mock",
        TransportChoice::Default => "default",
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::ReplyLiterals { model, clock, uuid } => {
                let identifier = Uuid::parse_str(uuid)
                    .map_err(|error| invalid(format!("uuid must be a UUID: {error}")))?;
                let raw = RawValue::from_string(serde_json::to_string(model).unwrap_or_default())
                    .map_err(|error| invalid(format!("model must be JSON: {error}")))?;
                let clock_value = *clock;
                let clock_fn = || clock_value;
                let identifier_fn = || identifier;
                let body = mock_chat_completion(MockModel::Raw(raw), &clock_fn, &identifier_fn);
                Ok(serde_json::to_value(body).unwrap_or(serde_json::Value::Null))
            }
            Input::RequestDecoder { probes } => {
                Ok(serde_json::Value::Array(probes.iter().map(decode_probe).collect()))
            }
            Input::Selection { scenarios } => Ok(serde_json::Value::Array(
                scenarios
                    .iter()
                    .map(|scenario| serde_json::Value::String(selection_outcome(scenario).to_owned()))
                    .collect(),
            )),
        }
    }
}

mod tests;
