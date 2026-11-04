use betterlite_openai::compatible_line_stream::{
    Mode, Outcome, ReceiveError, decode_legacy_compatible_line, receive_failure,
};
use serde::{Deserialize, Serialize};
use std::io;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StreamInput {
    mode: Mode,
    events: Vec<StreamEvent>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StreamEvent {
    Line { line: String },
    ValueError { message: String },
    ReceiveError { message: String },
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum Recorded {
    Line(Outcome),
    Error { kind: &'static str, value: String },
}

impl crate::helpers::Helper for Op {
    type Input = StreamInput;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        serde_json::to_value(run_stream(input)).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "SerializationError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}

fn run_stream(input: &StreamInput) -> Vec<Recorded> {
    let mut output = Vec::new();
    for event in &input.events {
        let failure = match event {
            StreamEvent::Line { line } => {
                let outcome = decode_legacy_compatible_line(line, input.mode);
                let end = outcome == Outcome::End;
                output.push(Recorded::Line(outcome));
                if end {
                    return output;
                }
                continue;
            }
            StreamEvent::ValueError { message } => {
                ReceiveError::Value(io::Error::other(message.clone()))
            }
            StreamEvent::ReceiveError { message } => {
                ReceiveError::Other(io::Error::other(message.clone()))
            }
        };
        output.push(Recorded::Error {
            kind: "error",
            value: receive_failure(failure, input.mode).to_string(),
        });
        return output;
    }
    output.push(Recorded::Line(Outcome::End));
    output
}

mod tests;
