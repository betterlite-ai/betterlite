use super::response;
use crate::model_armor::decisions::extract_content_from_response;

#[test]
fn extract_content_from_response_joins_every_choice_content() {
    let payload = response(
        r#"{"choices":[{"message":{"role":"assistant","content":"one"}},{"message":{"role":"assistant","content":"two"}}]}"#,
    );
    assert_eq!(extract_content_from_response(&payload), "onetwo");
}

#[test]
fn extract_content_from_response_appends_the_tool_call_json() {
    let payload = response(
        r#"{"choices":[{"message":{"content":"call:","tool_calls":[{"id":"1","type":"function"},{"id":"2","type":"function"}]}}]}"#,
    );
    assert_eq!(
        extract_content_from_response(&payload),
        r#"call:{"id":"1","type":"function"}{"id":"2","type":"function"}"#
    );
}

#[test]
fn extract_content_from_response_appends_the_function_call_json() {
    let payload = response(
        r#"{"choices":[{"message":{"content":"","function_call":{"name":"f","arguments":"{}"}}}]}"#,
    );
    assert_eq!(
        extract_content_from_response(&payload),
        r#"{"arguments":"{}","name":"f"}"#
    );
}

#[test]
fn extract_content_from_response_skips_empty_or_null_parts() {
    let null_call = response(r#"{"choices":[{"message":{"content":"hi","function_call":null}}]}"#);
    assert_eq!(extract_content_from_response(&null_call), "hi");
    let empty_calls = response(r#"{"choices":[{"message":{"content":"hi","tool_calls":[]}}]}"#);
    assert_eq!(extract_content_from_response(&empty_calls), "hi");
    let null_content = response(r#"{"choices":[{"message":{"role":"assistant","content":null}}]}"#);
    assert_eq!(extract_content_from_response(&null_content), "");
}

#[test]
fn extract_content_from_response_returns_empty_for_non_text_responses() {
    assert_eq!(
        extract_content_from_response(&response(r#"{"audio":"bytes","id":"tts"}"#)),
        ""
    );
    assert_eq!(
        extract_content_from_response(&response(r#"{"data":[{"b64_json":"image"}]}"#)),
        ""
    );
    assert_eq!(
        extract_content_from_response(&response(r#"{"choices":[{"text":"completion"}]}"#)),
        ""
    );
    assert_eq!(
        extract_content_from_response(&response(r#"{"choices":[{"message":"text"}]}"#)),
        ""
    );
    assert_eq!(
        extract_content_from_response(&response(r#"{"choices":"none"}"#)),
        ""
    );
    assert_eq!(extract_content_from_response(&response("{}")), "");
    assert_eq!(
        extract_content_from_response(&response(r#"["not","an","object"]"#)),
        ""
    );
}
