use betterlite_openai::compatible_line_stream::lines::{
    AsyncLines, ChunkItem, LineItem, SyncLines,
};
use betterlite_openai::compatible_line_stream::{Mode, Outcome, ReceiveError};
use futures_util::{StreamExt as _, stream};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
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
        let recorded = run_stream(input)?;
        serde_json::to_value(recorded).map_err(|error| helper_error("SerializationError", &error))
    }
}

fn helper_error(class: &str, error: &dyn std::fmt::Display) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn source(events: &[StreamEvent]) -> Vec<LineItem<io::Error>> {
    events
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
        .collect()
}

// LiteLLM json.dumps an object argument with its own spacing, so both sides compare the compact form
fn canonical_arguments(arguments: &str) -> String {
    serde_json::from_str::<Map<String, Value>>(arguments).map_or_else(
        |_| arguments.to_owned(),
        |object| Value::Object(object).to_string(),
    )
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

fn run_stream(input: &StreamInput) -> Result<Vec<Recorded>, Box<crate::helpers::HelperError>> {
    let lines = source(&input.events);
    let mut output: Vec<Recorded> = match input.mode {
        Mode::Sync => SyncLines::from(lines.into_iter()).map(record).collect(),
        Mode::Async => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .map_err(|error| helper_error("RuntimeError", &error))?;
            runtime.block_on(async {
                let mut chunks = AsyncLines::from(stream::iter(lines));
                let mut recorded = Vec::new();
                while let Some(item) = chunks.next().await {
                    recorded.push(record(item));
                }
                recorded
            })
        }
    };
    if !matches!(output.last(), Some(Recorded::Error { .. })) {
        output.push(Recorded::Line(Outcome::End));
    }
    Ok(output)
}

mod tests;
