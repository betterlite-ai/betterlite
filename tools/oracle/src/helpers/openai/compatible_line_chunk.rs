use betterlite_openai::compatible_line_stream::lines::{ChunkItem, LineItem, Lines};
use betterlite_openai::compatible_line_stream::{Mode, Outcome, ReceiveError};
use futures_util::{FutureExt as _, StreamExt as _, stream};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{io, iter};

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

// LiteLLM json.dumps an object argument with its own spacing, so both sides compare the compact form
fn canonical_arguments(arguments: &str) -> String {
    serde_json::from_str::<Value>(arguments)
        .map_or_else(|_| arguments.to_owned(), |parsed| parsed.to_string())
}

fn record(item: ChunkItem<io::Error>) -> Recorded {
    match item {
        Ok(mut chunk) => {
            if let Some(tool) = chunk.tool_use.as_mut() {
                tool.function.arguments = canonical_arguments(&tool.function.arguments);
            }
            Recorded::Line(Outcome::Chunk(chunk))
        }
        Err(error) => Recorded::Error {
            kind: "error",
            value: error.to_string(),
        },
    }
}

fn run_stream(input: &StreamInput) -> Vec<Recorded> {
    let lines: Vec<LineItem<io::Error>> = input
        .events
        .iter()
        .map(|event| match event {
            StreamEvent::Line { line } => Ok(line.clone()),
            StreamEvent::ValueError { message } => {
                Err(ReceiveError::Value(io::Error::other(message.clone())))
            }
            StreamEvent::ReceiveError { message } => {
                Err(ReceiveError::Other(io::Error::other(message.clone())))
            }
        })
        .collect();
    let mut output: Vec<Recorded> = match input.mode {
        Mode::Sync => Lines::from(lines.into_iter()).map(record).collect(),
        Mode::Async => {
            let mut chunks = Lines::from(stream::iter(lines));
            iter::from_fn(|| chunks.next().now_or_never().flatten())
                .map(record)
                .collect()
        }
    };
    if !matches!(output.last(), Some(Recorded::Error { .. })) {
        output.push(Recorded::Line(Outcome::End));
    }
    output
}

mod tests;
