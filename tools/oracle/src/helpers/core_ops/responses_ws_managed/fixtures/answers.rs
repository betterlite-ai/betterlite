use std::collections::BTreeMap;

use betterlite_oracle_runner::case::UpstreamAnswer;
use betterlite_oracle_runner::error::HelperRunError;
use betterlite_oracle_runner::mock::{MockProvider, ProviderCall};
use betterlite_oracle_runner::routing_mock_scripts::MockScript;
use serde_json::{Value, json};

const CREATED: &str = r#"{"type":"response.created","response":{"id":"resp_managed","object":"response","created_at":1741476542,"model":"gpt-4o-mini","status":"in_progress","output":[]}}"#;
const COMPLETED: &str = r#"{"type":"response.completed","response":{"id":"resp_managed","object":"response","created_at":1741476542,"model":"gpt-4o-mini","status":"completed","output":[]}}"#;

// the fixture dispatch always asks for streamed events, so every call is answered with one stream
pub(in crate::helpers::core_ops::responses_ws_managed) fn start_mock(
    slots: usize,
) -> Result<MockProvider, HelperRunError> {
    let answers = vec![answer(); slots];
    let script = MockScript::try_from(answers.as_slice())?;
    MockProvider::start(script)
}

fn answer() -> UpstreamAnswer {
    let mut headers = BTreeMap::new();
    headers.insert(
        String::from("content-type"),
        String::from("text/event-stream"),
    );
    UpstreamAnswer {
        status: 200,
        headers,
        body: None,
        events: Some(vec![String::from(CREATED), String::from(COMPLETED)]),
    }
}

pub(in crate::helpers::core_ops::responses_ws_managed) fn requests(
    calls: &[ProviderCall],
) -> Value {
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
