#![cfg(test)]

use crate::invoke_response::body;
use crate::invoke_response::tests::counter::FixtureCounter;
use crate::invoke_response::tests::support::{MODEL, decode, decode_with, fixture, json};

pub(in crate::invoke_response::tests) fn decode_and_processing_errors() {
    decode_failures_keep_the_raw_response();
    invalid_utf8_becomes_replacement_characters();
    malformed_family_payloads_are_processing_errors();
    debug_log_uses_four_spaces();
}

fn decode_failures_keep_the_raw_response() {
    let mut case = fixture();
    case.headers = vec![(String::from("x-amzn-requestid"), String::from("req-1"))];
    let run = decode(&case, "not json");
    assert_eq!(run.status(), Some(200));
    assert_eq!(run.message(), "not json");
    assert_eq!(run.response_headers(), case.headers.as_slice());

    case.status = 400;
    let truncated = r#"{"results": [{"outputText": "cut"#;
    let run = decode(&case, truncated);
    assert_eq!(run.status(), Some(400));
    assert_eq!(run.message(), truncated);
}

fn invalid_utf8_becomes_replacement_characters() {
    let case = fixture();
    let counter = FixtureCounter::counting(1, 1);
    let run = decode_with(&case, b"{\"results\": [\xff]}", &counter);
    assert_eq!(run.status(), Some(200));
    assert_eq!(run.message(), "{\"results\": [\u{FFFD}]}");
}

fn malformed_family_payloads_are_processing_errors() {
    let cases = [
        (
            MODEL,
            r#"{"other": 1}"#,
            "the results field is missing or malformed",
        ),
        (
            MODEL,
            r#"{"results": 5}"#,
            "the results field is missing or malformed",
        ),
        (
            MODEL,
            r#"{"results": []}"#,
            "the results field is missing or malformed",
        ),
        (MODEL, r#"[1, 2]"#, "the response body is not a JSON object"),
        (
            "ai21.j2-mid-v1",
            r#"{}"#,
            "the completions field is missing or malformed",
        ),
        (
            "meta.llama3-70b-instruct-v1:0",
            r#"{}"#,
            "the generation field is missing or malformed",
        ),
        (
            "cohere.command-r-v1:0",
            r#"{"generations": []}"#,
            "the generations field is missing or malformed",
        ),
        (
            "cohere.command-r-v1:0",
            r#"{"generations": [{"finish_reason": "COMPLETE"}]}"#,
            "the generations.text field is missing or malformed",
        ),
        (
            "cohere.command-r-v1:0",
            r#"{"generations": [{"text": "x", "finish_reason": ["a"]}]}"#,
            "the generations.finish_reason field has a type its declared shape does not allow",
        ),
        (
            "mistral.mistral-large-2402-v1:0",
            r#"{"wrong": true}"#,
            "Unexpected mistral completion response",
        ),
        (
            "mistral.mistral-large-2402-v1:0",
            r#"{"choices": [{"message": {"content": "x"}}]}"#,
            "the choices.finish_reason field is missing or malformed",
        ),
    ];
    for (model, payload, expected) in cases {
        let mut case = fixture();
        case.model = model.to_owned();
        let run = decode(&case, payload);
        assert_eq!(run.status(), Some(422), "{model}: {payload}");
        assert_eq!(
            run.message(),
            format!("Error processing={payload}, Received error={expected}"),
            "{model}: {payload}"
        );
    }
}

fn debug_log_uses_four_spaces() {
    let record = json(r#"{"a": 1, "b": [true, null]}"#);
    assert_eq!(
        body::rendered(&record).as_deref(),
        Some("{\n    \"a\": 1,\n    \"b\": [\n        true,\n        null\n    ]\n}")
    );
}
