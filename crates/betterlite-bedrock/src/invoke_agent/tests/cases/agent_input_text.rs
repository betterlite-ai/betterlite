use betterlite_logging::python_json::dumps_object;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::invoke_agent::error::InvokeAgentRequestError;
use crate::invoke_agent::payload::transform_request;
use crate::invoke_agent::tests::support::{messages, params, request_data};
use crate::model_routes::ParamMap;

const TEXT: &str = r#"[{"role":"user","content":"hi"}]"#;

pub(super) fn agent_input_text() {
    assert_defaults();
    assert_last_message_only();
    assert_content_parts();
    assert_search_results();
    assert_overlay();
    assert_empty_messages();
    assert_missing_content();
}

fn payload(message_json: &str, optional_params: &ParamMap) -> IndexMap<String, Json> {
    transform_request(&messages(message_json), optional_params).unwrap()
}

fn body(message_json: &str, optional_params: &ParamMap) -> String {
    dumps_object(&payload(message_json, optional_params))
}

fn assert_defaults() {
    assert_eq!(
        payload(TEXT, &params("{}")),
        request_data(r#"{"inputText":"hi","enableTrace":true}"#)
    );
}

fn assert_last_message_only() {
    assert_eq!(
        body(
            r#"[{"role":"system","content":"ignore me"},{"role":"user","content":"first"},{"role":"assistant","content":[{"type":"text","text":"second"}]}]"#,
            &params("{}")
        ),
        r#"{"inputText": "second", "enableTrace": true}"#
    );
}

fn assert_content_parts() {
    let message = r#"[{"role":"user","content":[{"type":"text","text":"first. "},{"type":"image_url","image_url":{"url":"https://example.test/a.png"}},{"type":"text","text":"second."}]}]"#;
    assert_eq!(
        payload(message, &params("{}")),
        request_data(r#"{"inputText":"first. second.","enableTrace":true}"#)
    );
}

fn assert_search_results() {
    let message = r#"[{"role":"user","content":"base","search_results":[{"source":"s","title":"t","content":[{"text":"c"},{"type":"image_url"}],"citations":{"a":1}}]}]"#;
    assert_eq!(
        payload(message, &params("{}")),
        request_data(r#"{"inputText":"basestc{\"a\":1}","enableTrace":true}"#)
    );
}

fn assert_overlay() {
    let optional = params(
        r#"{"inputText":"override","enableTrace":false,"sessionID":"s","aws_access_key_id":"AKID"}"#,
    );
    assert_eq!(
        body(TEXT, &optional),
        r#"{"inputText": "override", "enableTrace": false, "sessionID": "s", "aws_access_key_id": "AKID"}"#
    );
    let nested = params(r#"{"meta":{"b":1,"a":2}}"#);
    assert_eq!(
        body(TEXT, &nested),
        r#"{"inputText": "hi", "enableTrace": true, "meta": {"b": 1, "a": 2}}"#,
        "a nested object keeps the caller's key order"
    );
}

fn assert_empty_messages() {
    assert!(matches!(
        transform_request(&[], &params("{}")),
        Err(InvokeAgentRequestError::EmptyMessages)
    ));
}

fn assert_missing_content() {
    for message in [
        r#"[{"role":"user"}]"#,
        r#"[{"role":"user","content":null}]"#,
        r#"[{"role":"user","content":[]}]"#,
    ] {
        assert_eq!(
            payload(message, &params("{}")),
            request_data(r#"{"inputText":"","enableTrace":true}"#),
            "{message}"
        );
    }
}
