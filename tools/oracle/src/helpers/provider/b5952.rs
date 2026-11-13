use betterlite_prompts::arguments::json::Json;
use betterlite_provider::sap_stream::{
    Mode, SapLineStream, SapStep, SapStreamError, add_stream_flag, event_to_chunk,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", content = "params", rename_all = "snake_case")]
pub(crate) enum Input {
    Event(EventInput),
    Lines(LinesInput),
    Flag(FlagInput),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EventInput {
    event: Json,
    now: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LinesInput {
    lines: Vec<String>,
    modes: Vec<Mode>,
    now: i64,
    failure: Option<SourceFailure>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SourceFailure {
    Read,
    Os,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FlagInput {
    body: Json,
}

struct SimulatedRead;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Event(params) => event_output(params),
            Input::Lines(params) => lines_output(params),
            Input::Flag(params) => flag_output(params),
        }
    }
}

fn event_output(input: &EventInput) -> HelperResult {
    let found = event_to_chunk(&input.event, input.now).map_err(|error| failure(&error))?;
    let value = match found {
        Some(chunk) => serde_json::to_value(chunk).map_err(|source| encode_error(&source))?,
        None => Value::Null,
    };
    Ok(json!({"chunk": value}))
}

fn lines_output(input: &LinesInput) -> HelperResult {
    let mut traces = Vec::new();
    for mode in &input.modes {
        traces.push(trace(input, *mode)?);
    }
    Ok(json!({"traces": traces}))
}

fn trace(input: &LinesInput, mode: Mode) -> Result<Value, Box<HelperError>> {
    let mut stream = SapLineStream {
        now: input.now,
        done: false,
    };
    let mut chunks = Vec::new();
    let mut failed = None;
    for line in &input.lines {
        if stream.done {
            break;
        }
        match stream.next_chunk(line, mode) {
            Ok(SapStep::Chunk(chunk)) => {
                chunks.push(serde_json::to_value(chunk).map_err(|source| encode_error(&source))?);
            }
            Ok(SapStep::Skipped | SapStep::End) => {}
            Err(error) => {
                failed = Some(json!({"status": status(&error)}));
                break;
            }
        }
    }
    if !stream.done {
        if input.failure.is_some() {
            if stream.read_failed(SimulatedRead, mode).is_err() {
                failed = Some(json!({"status": null}));
            }
        } else {
            let _closed = stream.finish();
        }
    }
    Ok(json!({"chunks": chunks, "failure": failed, "done": stream.done}))
}

fn flag_output(input: &FlagInput) -> HelperResult {
    let mut body = input.body.clone();
    add_stream_flag(&mut body).map_err(|error| failure(&error))?;
    let value = serde_json::to_value(body).map_err(|source| encode_error(&source))?;
    Ok(json!({"body": value}))
}

const fn status(error: &SapStreamError) -> Option<u16> {
    match error {
        SapStreamError::Orchestration(orchestration) => Some(orchestration.status_code),
        SapStreamError::Event { .. }
        | SapStreamError::Chunk { .. }
        | SapStreamError::EventField(_)
        | SapStreamError::Timestamp { .. }
        | SapStreamError::ConfigBody
        | SapStreamError::StreamBody => None,
    }
}

fn failure(error: &SapStreamError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("SapStreamError"),
        status: status(error),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn encode_error(source: &serde_json::Error) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("EncodeError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: source.to_string(),
    })
}
