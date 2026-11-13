#![cfg(test)]

use std::sync::Mutex;

use betterlite_prompts::arguments::json::{Json, Object};
use chrono::DateTime;

use super::{AuthorizedToolCall, ProgressCallback};

#[derive(Default)]
struct Recording {
    seen: Mutex<Vec<(f64, Option<f64>)>>,
}

impl ProgressCallback for Recording {
    async fn report(&self, progress: f64, total: Option<f64>) {
        self.seen.lock().unwrap().push((progress, total));
    }
}

#[tokio::test]
async fn an_authorized_tool_call_holds_the_request_and_forwards_progress() {
    let call = AuthorizedToolCall {
        name: "github-create_issue".to_owned(),
        arguments: Object::from([("title".to_owned(), Json::Text("bug".to_owned()))]),
        allowed_mcp_servers: Vec::new(),
        start_time: DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
        host_progress_callback: Some(Recording::default()),
        guardrail_context: None,
        logging_data: Object::from([("call_type".to_owned(), Json::Text("mcp".to_owned()))]),
    };
    assert_eq!(call.name, "github-create_issue");
    assert_eq!(
        call.arguments.get("title"),
        Some(&Json::Text("bug".to_owned()))
    );
    assert!(call.allowed_mcp_servers.is_empty());
    assert!(call.guardrail_context.is_none());
    assert_eq!(call.logging_data.len(), 1);
    let callback = call.host_progress_callback.unwrap();
    callback.report(0.5_f64, Some(1.0_f64)).await;
    callback.report(1.0, None).await;
    let seen = callback.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0], (0.5_f64, Some(1.0_f64)));
    assert_eq!(seen[1], (1.0_f64, None));
}
