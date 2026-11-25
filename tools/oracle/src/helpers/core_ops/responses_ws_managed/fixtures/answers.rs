use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_oracle_runner::case::UpstreamAnswer;
use betterlite_oracle_runner::error::HelperRunError;
use betterlite_oracle_runner::mock::MockProvider;
use betterlite_oracle_runner::routing_mock_scripts::MockScript;
use betterlite_oracle_runner::routing_mock_scripts::ProviderCall;
use serde_json::{Value, json};

use super::super::Input;
use super::super::support::ScriptRow;
use super::rows::ChunkKind;

pub(crate) fn start_mock(input: &Input) -> Result<MockProvider, HelperRunError> {
    let streaming = input.setup.kwargs.get("stream") != Some(&Json::Bool(false));
    let answers = input
        .scripts
        .iter()
        .map(|script| answer(script, streaming))
        .collect::<Vec<_>>();
    let script = MockScript::try_from(answers.as_slice())?;
    MockProvider::start(script)
}

fn answer(script: &ScriptRow, streaming: bool) -> UpstreamAnswer {
    let mut headers = BTreeMap::new();
    let content_type = if streaming {
        "text/event-stream"
    } else {
        "application/json"
    };
    headers.insert(String::from("content-type"), String::from(content_type));
    UpstreamAnswer {
        status: 200,
        headers,
        body: (!streaming).then(|| json!({"id":"buffered","created_at":0_i64,"output":[],"model":"gpt-4o-mini","object":"response"})),
        events: streaming.then(|| script_events(script)),
    }
}

fn script_events(script: &ScriptRow) -> Vec<String> {
    if let Some(message) = &script.fail {
        return vec![json!({"type":"fixture.error","message":message}).to_string()];
    }
    let mut events = Vec::new();
    for (index, chunk) in script.chunks.iter().enumerate() {
        if script.raise_after == Some(index) {
            events.push(json!({"type":"fixture.error","message":script.raise_message}).to_string());
            return events;
        }
        let event = match chunk.kind {
            ChunkKind::Json => json!(chunk.value).to_string(),
            ChunkKind::Object => chunk
                .dump
                .clone()
                .unwrap_or_else(|| json!({"type":chunk.event_type}).to_string()),
            ChunkKind::None => json!({"type":"fixture.empty"}).to_string(),
            ChunkKind::Broken => json!({"type":"fixture.broken"}).to_string(),
        };
        events.push(event);
    }
    events.push(json!({"type":"fixture.end"}).to_string());
    events
}

pub(crate) fn requests(calls: &[ProviderCall]) -> Value {
    let rows: Vec<Value> = calls
        .iter()
        .map(|call| {
            json!({
                "method": call.request.method,
                "path": call.request.path,
                "body": call.request.body,
            })
        })
        .collect();
    Value::Array(rows)
}
