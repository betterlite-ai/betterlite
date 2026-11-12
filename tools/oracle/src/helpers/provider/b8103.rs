use std::collections::VecDeque;
use std::fmt::Display;
use std::sync::Arc;

use betterlite_http::error::HttpError;
use betterlite_openai::aleph_alpha::lines::LineDecoder;
use betterlite_openai::stream::chunk::{Choice, Chunk};
use betterlite_provider::langgraph_stream::stream::{LangGraphSseStream, LineReader};
use betterlite_provider::langgraph_stream::{IdSource, MAX_LINE_BYTES};
use futures_util::StreamExt as _;
use futures_util::stream;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Deserialize)]
struct StreamCase {
    model: String,
    lines: Vec<String>,
}

#[derive(Deserialize)]
pub(crate) struct Input {
    cases: Vec<StreamCase>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|source| helper_error("RuntimeError", &source))?;
        let mut cases = Vec::with_capacity(input.cases.len());
        for case in &input.cases {
            let sync = runtime.block_on(chunks(case))?;
            let asynchronous = runtime.block_on(chunks(case))?;
            cases.push(json!({"sync": sync, "async": asynchronous}));
        }
        Ok(json!({"cases": cases}))
    }
}

async fn chunks(case: &StreamCase) -> Result<Value, Box<HelperError>> {
    let body = case.lines.join("\n");
    let frames = vec![Ok::<Vec<u8>, HttpError>(body.into_bytes())];
    let mut stream = LangGraphSseStream {
        model: case.model.clone(),
        ids: Arc::new(FixedIds),
        lines: Some(LineReader {
            chunks: stream::iter(frames).boxed(),
            decoder: LineDecoder::new(MAX_LINE_BYTES),
            lines: VecDeque::new(),
        }),
        finished: false,
    };
    let mut found = Vec::new();
    while let Some(chunk) = stream.next_chunk().await {
        found.push(view(&chunk));
    }
    Ok(Value::Array(found))
}

struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        Uuid::nil()
    }
}

fn view(chunk: &Chunk) -> Value {
    json!({
        "created": chunk.created,
        "model": chunk.model,
        "object": chunk.object,
        "choices": chunk.choices.iter().map(choice_view).collect::<Vec<Value>>(),
    })
}

fn choice_view(choice: &Choice) -> Value {
    json!({
        "index": choice.index,
        "finish_reason": choice.finish_reason,
        "delta": {"content": choice.delta.content, "role": choice.delta.role},
    })
}

fn helper_error<E>(class: &str, error: &E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
