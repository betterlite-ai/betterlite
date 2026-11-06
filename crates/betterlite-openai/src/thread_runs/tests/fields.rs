use serde_json::Value;

use crate::thread_runs::records::RunEvent;

use super::cases::{frame, stream_result};
use super::support::parse;

const STEP_CREATED: &str = r#"{"id":"step_1","object":"thread.run.step","created_at":1700000000,"run_id":"run_1","assistant_id":"asst_1","thread_id":"thread_a","type":"tool_calls","status":"in_progress","step_details":{"type":"tool_calls","tool_calls":[]}}"#;
const STEP_DELTA_FUNCTION: &str = r#"{"id":"step_1","object":"thread.run.step.delta","delta":{"step_details":{"type":"tool_calls","tool_calls":[{"index":0,"type":"function","function":{"arguments":"{"}}]}}}"#;
const STEP_FUNCTION: &str = r#"{"id":"step_1","object":"thread.run.step","created_at":1700000000,"run_id":"run_1","assistant_id":"asst_1","thread_id":"thread_a","type":"tool_calls","status":"completed","step_details":{"type":"tool_calls","tool_calls":[{"index":0,"type":"function","id":"call_1","function":{"name":"f","arguments":"{}"}}]}}"#;
const MESSAGE_CREATED: &str = r#"{"id":"msg_1","object":"thread.message","created_at":1700000000,"thread_id":"thread_a","role":"assistant","status":"in_progress","content":[]}"#;
const MESSAGE_DELTA_TEXT: &str = r#"{"id":"msg_1","object":"thread.message.delta","delta":{"content":[{"index":0,"type":"text","text":{"annotations":[]}}]}}"#;
const MESSAGE_DELTA_IMAGE_FILE: &str = r#"{"id":"msg_1","object":"thread.message.delta","delta":{"content":[{"index":0,"type":"image_file","image_file":{"file_id":"file_1"}}]}}"#;
const MESSAGE_DELTA_IMAGE_URL: &str = r#"{"id":"msg_1","object":"thread.message.delta","delta":{"content":[{"index":0,"type":"image_url","image_url":{"url":"https://example.test/x.png"}}]}}"#;
const MESSAGE_IMAGE_FILE: &str = r#"{"id":"msg_1","object":"thread.message","created_at":1700000000,"thread_id":"thread_a","role":"assistant","status":"completed","content":[{"index":0,"type":"image_file","image_file":{"file_id":"file_1"}}]}"#;
const THREAD_CREATED: &str = r#"{"id":"thread_a","object":"thread","created_at":1700000000,"metadata":{},"tool_resources":{"code_interpreter":{"file_ids":[]}}}"#;

// A field the SDK cannot use never clears a data line recorded before it.
pub(super) async fn stream_unusable_fields_after_data_keep_the_event() {
    for body in [
        "data: {\"a\":1}\nretry: soon\n\n",
        "retry: soon\ndata: {\"a\":1}\n\n",
        "data: {\"a\":1}\nid:\n\n",
        "data: {\"a\":1}\nevent:\n\n",
        "data: {\"a\":1}\nid: a\0b\n\n",
    ] {
        let events = stream_result(body).await.unwrap();
        assert_eq!(events.len(), 1, "{body:?}");
        assert_eq!(events[0].name(), "payload");
        assert_eq!(
            serde_json::to_value(&events[0]).unwrap(),
            parse(r#"{"a":1}"#)
        );
    }
}

// Nested step and message records keep every declared key the provider left out as null.
pub(super) async fn stream_nested_records_match_the_sdk_dump() {
    let events = nested_events().await;
    assert_step_records(&events);
    assert_message_records(&events);
    assert_thread_record(&events);
}

async fn nested_events() -> Vec<RunEvent> {
    let body = format!(
        "{}{}{}{}{}{}{}{}{}data: [DONE]\n\n",
        frame("thread.run.step.created", STEP_CREATED),
        frame("thread.run.step.delta", STEP_DELTA_FUNCTION),
        frame("thread.run.step.completed", STEP_FUNCTION),
        frame("thread.message.created", MESSAGE_CREATED),
        frame("thread.message.delta", MESSAGE_DELTA_TEXT),
        frame("thread.message.delta", MESSAGE_DELTA_IMAGE_FILE),
        frame("thread.message.delta", MESSAGE_DELTA_IMAGE_URL),
        frame("thread.message.completed", MESSAGE_IMAGE_FILE),
        frame("thread.created", THREAD_CREATED),
    );
    let events = stream_result(&body).await.unwrap();
    assert_eq!(events.len(), 9);
    events
}

fn assert_step_records(events: &[RunEvent]) {
    assert!(events.len() > 2);
    let delta = serde_json::to_value(&events[1]).unwrap();
    assert_eq!(
        delta.pointer("/data/delta/step_details/tool_calls/0/id"),
        Some(&Value::Null)
    );
    assert_eq!(
        delta.pointer("/data/delta/step_details/tool_calls/0/function/name"),
        Some(&Value::Null)
    );
    assert_eq!(
        delta.pointer("/data/delta/step_details/tool_calls/0/function/output"),
        Some(&Value::Null)
    );
    assert_eq!(
        delta.pointer("/data/delta/step_details/tool_calls/0/function/arguments"),
        Some(&Value::String(String::from("{")))
    );
    let step = serde_json::to_value(&events[2]).unwrap();
    assert_eq!(
        step.pointer("/data/step_details/tool_calls/0/function/output"),
        Some(&Value::Null)
    );
}

fn assert_message_records(events: &[RunEvent]) {
    assert!(events.len() > 7);
    let text = serde_json::to_value(&events[4]).unwrap();
    assert_eq!(
        text.pointer("/data/delta/content/0/text/value"),
        Some(&Value::Null)
    );
    let image_delta = serde_json::to_value(&events[5]).unwrap();
    assert_eq!(
        image_delta.pointer("/data/delta/content/0/image_file/detail"),
        Some(&Value::Null)
    );
    let url_delta = serde_json::to_value(&events[6]).unwrap();
    assert_eq!(
        url_delta.pointer("/data/delta/content/0/image_url/detail"),
        Some(&Value::Null)
    );
    let image = serde_json::to_value(&events[7]).unwrap();
    assert_eq!(
        image.pointer("/data/content/0/image_file/detail"),
        Some(&Value::Null)
    );
}

fn assert_thread_record(events: &[RunEvent]) {
    assert!(events.len() > 8);
    let thread = serde_json::to_value(&events[8]).unwrap();
    assert_eq!(
        thread.pointer("/data/tool_resources/file_search"),
        Some(&Value::Null)
    );
    assert_eq!(
        thread.pointer("/data/tool_resources/code_interpreter/file_ids"),
        Some(&Value::Array(Vec::new()))
    );
}
