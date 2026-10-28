use betterlite_logging::message_content::content_text;
use betterlite_logging::otlp_normalize::truncate;
use betterlite_logging::otlp_payload::truncate_payload;
use betterlite_logging::otlp_response::encode_otlp_response;
use betterlite_logging::ui_content::to_ui_content;
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

// Cases repeat a piece so a 64 KiB value stays a short line in the case file
#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum Piece {
    Text(String),
    Repeat { text: String, times: usize },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Truncate {
        value: Vec<Piece>,
    },
    Elision {
        value: Vec<Piece>,
    },
    // JSON text rather than a value, so object keys reach both sides in the order the case wrote them
    ContentText {
        content: String,
    },
    UiContent {
        value: Vec<Piece>,
    },
    ResponseEncoding {
        content_type: Option<String>,
        error: Option<String>,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Truncate { value } => Ok(json!(truncate(&joined(value)))),
            Input::Elision { value } => Ok(json!(truncate_payload(&joined(value)))),
            Input::ContentText { content } => {
                let parsed: Json = serde_json::from_str(content).map_err(|error| invalid(&error))?;
                Ok(json!(content_text(&parsed)))
            }
            Input::UiContent { value } => serde_json::to_value(to_ui_content(&joined(value)))
                .map_err(|error| invalid(&error)),
            Input::ResponseEncoding {
                content_type,
                error,
            } => {
                let (body, media_type) =
                    encode_otlp_response(content_type.as_deref(), error.as_deref());
                Ok(json!({"body": body, "content_type": media_type}))
            }
        }
    }
}

fn joined(pieces: &[Piece]) -> String {
    let mut value = String::new();
    for piece in pieces {
        match piece {
            Piece::Text(text) => value.push_str(text),
            Piece::Repeat { text, times } => value.push_str(&text.repeat(*times)),
        }
    }
    value
}

fn invalid(error: &serde_json::Error) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
