#![cfg(test)]

use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;

use crate::purview_dlp_evaluation::stream_assembly::assemble_responses_stream;

fn chunks(raw: &str) -> Vec<ResponsesAPIStreamingResponse> {
    serde_json::from_str(raw).unwrap()
}

fn response_event(kind: &str, id: &str) -> String {
    format!(r#"{{"type":"{kind}","response":{{"id":"{id}","created_at":1,"output":[]}}}}"#)
}

#[test]
fn assembles_the_response_completed_event() {
    let stream = chunks(&format!(
        "[{}]",
        response_event("response.completed", "resp-done")
    ));
    let (is_responses_api, assembled) = assemble_responses_stream(&stream);
    assert!(is_responses_api);
    assert_eq!(assembled.unwrap().id, "resp-done");
}

#[test]
fn falls_back_to_the_response_failed_event() {
    let stream = chunks(&format!(
        "[{}]",
        response_event("response.failed", "resp-failed")
    ));
    let (is_responses_api, assembled) = assemble_responses_stream(&stream);
    assert!(is_responses_api);
    assert_eq!(assembled.unwrap().id, "resp-failed");
}

#[test]
fn falls_back_to_the_response_incomplete_event() {
    let stream = chunks(&format!(
        "[{}]",
        response_event("response.incomplete", "resp-incomplete")
    ));
    let (is_responses_api, assembled) = assemble_responses_stream(&stream);
    assert!(is_responses_api);
    assert_eq!(assembled.unwrap().id, "resp-incomplete");
}

#[test]
fn keeps_the_latest_response_carrying_event() {
    let stream = chunks(&format!(
        "[{},{}]",
        response_event("response.created", "resp-early"),
        response_event("response.in_progress", "resp-late")
    ));
    let (is_responses_api, assembled) = assemble_responses_stream(&stream);
    assert!(is_responses_api);
    assert_eq!(assembled.unwrap().id, "resp-late");
}

#[test]
fn a_responses_stream_without_a_response_body_reports_no_final_response() {
    let stream = chunks(
        r#"[{"type":"response.output_text.delta","item_id":"item","output_index":0,"content_index":0,"delta":"hi"}]"#,
    );
    let (is_responses_api, assembled) = assemble_responses_stream(&stream);
    assert!(is_responses_api);
    assert!(assembled.is_none());
}

#[test]
fn an_unknown_response_event_type_still_marks_a_responses_stream() {
    let stream = chunks(
        r#"[{"type":"response.queued","response":{"id":"resp-queued","created_at":1,"output":[]}}]"#,
    );
    let (is_responses_api, assembled) = assemble_responses_stream(&stream);
    assert!(is_responses_api);
    assert!(assembled.is_none());
}

#[test]
fn a_typed_error_event_is_not_a_responses_stream() {
    let stream = chunks(
        r#"[{"type":"error","sequence_number":1,"error":{"type":"invalid_request_error","code":"bad_request","message":"boom"}}]"#,
    );
    let (is_responses_api, assembled) = assemble_responses_stream(&stream);
    assert!(!is_responses_api);
    assert!(assembled.is_none());
}

#[test]
fn a_chat_stream_is_not_a_responses_stream() {
    let stream = chunks(r#"[{"type":"chat.completion.chunk","choices":[]}]"#);
    let (is_responses_api, assembled) = assemble_responses_stream(&stream);
    assert!(!is_responses_api);
    assert!(assembled.is_none());
}

#[test]
fn an_error_event_is_not_a_responses_stream() {
    let stream = chunks(r#"[{"type":"error","message":"boom"}]"#);
    let (is_responses_api, assembled) = assemble_responses_stream(&stream);
    assert!(!is_responses_api);
    assert!(assembled.is_none());
}

#[test]
fn an_empty_stream_has_no_responses_answer() {
    let (is_responses_api, assembled) = assemble_responses_stream(&[]);
    assert!(!is_responses_api);
    assert!(assembled.is_none());
}
