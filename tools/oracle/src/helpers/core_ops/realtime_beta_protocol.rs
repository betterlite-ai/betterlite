use betterlite_core::api::realtime_beta_protocol::events::translate_event_to_beta;
use betterlite_core::api::realtime_beta_protocol::headers::{
    HeaderField, HeaderScope, openai_beta_realtime_header,
};
use betterlite_core::api::realtime_beta_protocol::session::remap_beta_session_to_ga;
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::json;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

type Failure = Box<HelperError>;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    HeaderDetection(HeaderInput),
    SessionRemap(SessionInput),
    GaEventsForBeta(EventsInput),
}

#[derive(Deserialize)]
pub(crate) struct HeaderInput {
    websockets: Vec<WebSocketInput>,
}

#[derive(Deserialize)]
pub(crate) struct WebSocketInput {
    headers: Vec<(String, String)>,
    latin1: bool,
    scope_error: bool,
}

#[derive(Deserialize)]
pub(crate) struct SessionInput {
    sessions: Vec<Json>,
}

#[derive(Deserialize)]
pub(crate) struct EventsInput {
    events: Vec<Json>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::HeaderDetection(fields) => {
                let answers = header_answers(&fields.websockets)?;
                Ok(json!({ "answers": answers }))
            }
            Input::SessionRemap(fields) => {
                let answers = session_answers(&fields.sessions)?;
                Ok(json!({ "answers": answers }))
            }
            Input::GaEventsForBeta(fields) => {
                let answers = event_answers(&fields.events)?;
                Ok(json!({ "answers": answers }))
            }
        }
    }
}

fn header_answers(websockets: &[WebSocketInput]) -> Result<Vec<bool>, Failure> {
    let mut answers = Vec::with_capacity(websockets.len());
    for websocket in websockets {
        answers.push(header_answer(websocket)?);
    }
    Ok(answers)
}

fn header_answer(websocket: &WebSocketInput) -> Result<bool, Failure> {
    if websocket.scope_error {
        return Ok(openai_beta_realtime_header(None));
    }
    let mut headers = Vec::with_capacity(websocket.headers.len());
    for (name, value) in &websocket.headers {
        headers.push((
            header_field(name, websocket.latin1)?,
            header_field(value, websocket.latin1)?,
        ));
    }
    let scope = HeaderScope {
        headers: headers.as_slice(),
    };
    Ok(openai_beta_realtime_header(Some(&scope)))
}

fn header_field(text: &str, latin1: bool) -> Result<HeaderField, Failure> {
    if !latin1 {
        return Ok(HeaderField::Text(text.to_owned()));
    }
    let bytes = text
        .chars()
        .map(|character| u8::try_from(u32::from(character)).ok())
        .collect::<Option<Vec<u8>>>()
        .ok_or_else(|| invalid_input(format!("{text} is not a Latin-1 string")))?;
    Ok(HeaderField::Bytes(bytes))
}

fn session_answers(sessions: &[Json]) -> Result<Vec<Json>, Failure> {
    let mut answers = Vec::with_capacity(sessions.len());
    for session in sessions {
        answers.push(remap_beta_session_to_ga(session.clone()).or_value_error()?);
    }
    Ok(answers)
}

fn event_answers(events: &[Json]) -> Result<Vec<Option<Json>>, Failure> {
    let mut answers = Vec::with_capacity(events.len());
    for event in events {
        answers.push(translate_event_to_beta(event.clone()).or_value_error()?);
    }
    Ok(answers)
}

fn invalid_input(message: String) -> Failure {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    })
}
