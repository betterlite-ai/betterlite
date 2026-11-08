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
const STEP_DELTA_FUNCTION_MISSING: &str = r#"{"id":"step_1","object":"thread.run.step.delta","delta":{"step_details":{"type":"tool_calls","tool_calls":[{"index":0,"type":"function","id":"c"}]}}}"#;
const STEP_DELTA_CODE_MISSING: &str = r#"{"id":"step_1","object":"thread.run.step.delta","delta":{"step_details":{"type":"tool_calls","tool_calls":[{"index":0,"type":"code_interpreter","id":"c"}]}}}"#;
const STEP_DELTA_MESSAGE_CREATION: &str = r#"{"id":"step_1","object":"thread.run.step.delta","delta":{"step_details":{"type":"message_creation","message_creation":{}}}}"#;
const STEP_DELTA_MESSAGE_CREATION_KEYLESS: &str = r#"{"id":"step_1","object":"thread.run.step.delta","delta":{"step_details":{"type":"message_creation"}}}"#;
const STEP_DELTA_TOOL_CALLS_KEYLESS: &str = r#"{"id":"step_1","object":"thread.run.step.delta","delta":{"step_details":{"type":"tool_calls"}}}"#;
const STEP_MESSAGE_CREATION: &str = r#"{"id":"step_1","object":"thread.run.step","created_at":1700000000,"run_id":"run_1","assistant_id":"asst_1","thread_id":"thread_a","type":"message_creation","status":"completed","step_details":{"type":"message_creation","message_creation":{}}}"#;
const MESSAGE_DELTA_PARTIAL_BLOCKS: &str = r#"{"id":"msg_1","object":"thread.message.delta","delta":{"content":[{"index":0,"type":"text"},{"index":1,"type":"refusal"},{"index":2,"type":"text","text":{"value":"hi","annotations":[{"index":0,"type":"file_citation","file_citation":{"file_id":"f"}},{"index":1,"type":"file_path","file_path":{}}]}}]}}"#;
const THREAD_EMPTY_RESOURCES: &str = r#"{"id":"thread_a","object":"thread","created_at":1700000000,"metadata":{},"tool_resources":{"code_interpreter":{},"file_search":{}}}"#;
const STEP_DELTA_FILE_SEARCH: &str = r#"{"id":"step_1","object":"thread.run.step.delta","delta":{"step_details":{"type":"tool_calls","tool_calls":[{"index":0,"type":"file_search","id":"c","file_search":{"x":1}}]}}}"#;
const STEP_FILE_SEARCH: &str = r#"{"id":"step_1","object":"thread.run.step","created_at":1700000000,"run_id":"run_1","assistant_id":"asst_1","thread_id":"thread_a","type":"tool_calls","status":"completed","step_details":{"type":"tool_calls","tool_calls":[{"index":0,"type":"file_search","id":"c","file_search":{}}]}}"#;
const MESSAGE_DELTA_UNKNOWN_BLOCK: &str = r#"{"id":"msg_1","object":"thread.message.delta","delta":{"content":[{"index":0,"type":"audio"}]}}"#;

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

// A later event or id line replaces the earlier one, so the frame is skipped and the next event arrives.
pub(super) async fn stream_replaced_fields_do_not_hold_the_frame() {
    let body = concat!(
        "event: x\nevent:\n\n",
        "data: {\"a\":1}\n\n",
        "id: 1\nid:\n\n",
        "data: {\"a\":2}\n\n",
        "event: thread.foo\nevent:\n\n",
        "data: {\"a\":3}\n\n",
        "data: [DONE]\n\n",
    );
    let events = stream_result(body).await.unwrap();
    assert_eq!(events.len(), 3);
    let payloads = [r#"{"a":1}"#, r#"{"a":2}"#, r#"{"a":3}"#];
    for (event, payload) in events.iter().zip(payloads) {
        assert_eq!(serde_json::to_value(event).unwrap(), parse(payload));
    }
}

// A comment-only frame after an id line is skipped; the SDK fails its empty-data JSON parse there.
pub(super) async fn stream_comment_after_id_is_skipped() {
    let body = "id: 1\ndata: {\"a\":1}\n\n: ping\n\ndata: {\"a\":2}\n\ndata: [DONE]\n\n";
    let events = stream_result(body).await.unwrap();
    assert_eq!(events.len(), 2);
    let payloads = vec![parse(r#"{"a":1}"#), parse(r#"{"a":2}"#)];
    let values: Vec<Value> = events
        .iter()
        .map(|event| serde_json::to_value(event).unwrap())
        .collect();
    assert_eq!(values, payloads);
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
    let pointers = [
        "/data/delta/content/0/text/value",
        "/data/delta/content/0/image_file/detail",
        "/data/delta/content/0/image_url/detail",
        "/data/content/0/image_file/detail",
    ];
    for (event, pointer) in events.iter().skip(4).zip(pointers) {
        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value.pointer(pointer), Some(&Value::Null), "{pointer}");
    }
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

// A variant object or annotated block the provider left out is null, as the SDK dump keeps it.
pub(super) async fn stream_missing_variant_keys_match_the_sdk_dump() {
    let events = missing_variant_events().await;
    assert_step_variants(&events);
    assert_content_variants(&events);
}

async fn missing_variant_events() -> Vec<RunEvent> {
    let frames = [
        ("thread.run.step.created", STEP_CREATED),
        ("thread.run.step.delta", STEP_DELTA_FUNCTION_MISSING),
        ("thread.run.step.delta", STEP_DELTA_CODE_MISSING),
        ("thread.run.step.delta", STEP_DELTA_MESSAGE_CREATION),
        ("thread.run.step.delta", STEP_DELTA_MESSAGE_CREATION_KEYLESS),
        ("thread.run.step.delta", STEP_DELTA_TOOL_CALLS_KEYLESS),
        ("thread.run.step.completed", STEP_MESSAGE_CREATION),
        ("thread.message.created", MESSAGE_CREATED),
        ("thread.message.delta", MESSAGE_DELTA_PARTIAL_BLOCKS),
        ("thread.created", THREAD_EMPTY_RESOURCES),
        ("thread.run.step.delta", STEP_DELTA_FILE_SEARCH),
        ("thread.run.step.completed", STEP_FILE_SEARCH),
        ("thread.message.delta", MESSAGE_DELTA_UNKNOWN_BLOCK),
    ];
    let mut body = String::new();
    for (name, data) in frames {
        body.push_str(&frame(name, data));
    }
    body.push_str("data: [DONE]\n\n");
    let events = stream_result(&body).await.unwrap();
    assert_eq!(events.len(), 13);
    events
}

fn data_value(event: &RunEvent) -> Value {
    let value = serde_json::to_value(event).unwrap();
    let data = value.get("data").cloned();
    data.unwrap_or_default()
}

fn assert_step_variants(events: &[RunEvent]) {
    assert!(events.len() > 11);
    let function = parse(
        r#"{"id":"step_1","object":"thread.run.step.delta","delta":{"step_details":{"type":"tool_calls","tool_calls":[{"index":0,"type":"function","id":"c","function":null}]}}}"#,
    );
    assert_eq!(data_value(&events[1]), function);
    let code = parse(
        r#"{"id":"step_1","object":"thread.run.step.delta","delta":{"step_details":{"type":"tool_calls","tool_calls":[{"index":0,"type":"code_interpreter","id":"c","code_interpreter":null}]}}}"#,
    );
    assert_eq!(data_value(&events[2]), code);
    let delta = parse(
        r#"{"id":"step_1","object":"thread.run.step.delta","delta":{"step_details":{"type":"message_creation","message_creation":{"message_id":null}}}}"#,
    );
    assert_eq!(data_value(&events[3]), delta);
    let creation = parse(
        r#"{"id":"step_1","object":"thread.run.step.delta","delta":{"step_details":{"type":"message_creation","message_creation":null}}}"#,
    );
    assert_eq!(data_value(&events[4]), creation);
    let calls = parse(
        r#"{"id":"step_1","object":"thread.run.step.delta","delta":{"step_details":{"type":"tool_calls","tool_calls":null}}}"#,
    );
    assert_eq!(data_value(&events[5]), calls);
    let step = parse(
        r#"{"id":"step_1","object":"thread.run.step","created_at":1700000000,"run_id":"run_1","assistant_id":"asst_1","thread_id":"thread_a","type":"message_creation","status":"completed","step_details":{"type":"message_creation","message_creation":{"message_id":null}},"cancelled_at":null,"completed_at":null,"expired_at":null,"failed_at":null,"last_error":null,"metadata":null,"usage":null}"#,
    );
    assert_eq!(data_value(&events[6]), step);
    let delta_search = parse(
        r#"{"id":"step_1","object":"thread.run.step.delta","delta":{"step_details":{"type":"tool_calls","tool_calls":[{"index":0,"type":"file_search","id":"c","file_search":{"x":1}}]}}}"#,
    );
    assert_eq!(data_value(&events[10]), delta_search);
    let step_search = parse(
        r#"{"id":"step_1","object":"thread.run.step","created_at":1700000000,"run_id":"run_1","assistant_id":"asst_1","thread_id":"thread_a","type":"tool_calls","status":"completed","step_details":{"type":"tool_calls","tool_calls":[{"index":0,"type":"file_search","id":"c","file_search":{"ranking_options":null,"results":null}}]},"cancelled_at":null,"completed_at":null,"expired_at":null,"failed_at":null,"last_error":null,"metadata":null,"usage":null}"#,
    );
    assert_eq!(data_value(&events[11]), step_search);
}

fn assert_content_variants(events: &[RunEvent]) {
    assert!(events.len() > 12);
    let blocks = serde_json::to_value(&events[8]).unwrap();
    for pointer in [
        "/data/delta/content/0/text",
        "/data/delta/content/1/refusal",
        "/data/delta/content/2/text/annotations/0/end_index",
        "/data/delta/content/2/text/annotations/0/file_citation/quote",
        "/data/delta/content/2/text/annotations/1/end_index",
        "/data/delta/content/2/text/annotations/1/file_path/file_id",
        "/data/delta/content/2/text/annotations/1/start_index",
        "/data/delta/content/2/text/annotations/1/text",
    ] {
        assert_eq!(blocks.pointer(pointer), Some(&Value::Null), "{pointer}");
    }
    let thread = serde_json::to_value(&events[9]).unwrap();
    for pointer in [
        "/data/tool_resources/code_interpreter/file_ids",
        "/data/tool_resources/file_search/vector_store_ids",
    ] {
        assert_eq!(thread.pointer(pointer), Some(&Value::Null), "{pointer}");
    }
    let unknown = parse(
        r#"{"id":"msg_1","object":"thread.message.delta","delta":{"content":[{"index":0,"type":"audio"}],"role":null}}"#,
    );
    assert_eq!(data_value(&events[12]), unknown);
}
