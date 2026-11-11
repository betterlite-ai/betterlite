use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::agentcore_request::error::AgentCoreRequestError;
use crate::agentcore_request::identifiers::{
    SESSION_HEADER, USER_HEADER, runtime_session_id as compute_session_id,
    runtime_user_id as compute_user_id,
};
use crate::agentcore_request::payload::transform_request;
use crate::agentcore_request::tests::support::{
    generated_uuid, header, headers, messages, params, request_data,
};
use crate::model_routes::ParamMap;

const GENERATED: &str = "litellm-session-123e4567-e89b-12d3-a456-426614174000";
const TEXT: &str = r#"[{"role":"user","content":"hi"}]"#;
const IMAGE: &str = r#"[{"role":"user","content":[{"type":"text","text":"summarize"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AAA"}}]}]"#;
const CONTENT: &str = r#"{"prompt":"summarize","content":[{"type":"text","text":"summarize"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AAA"}}]}"#;

fn payload_for(
    message_json: &str,
    optional: &ParamMap,
    litellm: &ParamMap,
) -> Result<IndexMap<String, Json>, AgentCoreRequestError> {
    let mut request_headers = headers(&[]);
    transform_request(
        &messages(message_json),
        optional,
        litellm,
        &mut request_headers,
        generated_uuid(),
    )
}

fn forwarded(optional: &ParamMap, litellm: &ParamMap) -> IndexMap<String, Json> {
    payload_for(IMAGE, optional, litellm).unwrap()
}

fn flag_value(flag: &str) -> ParamMap {
    params(&format!(r#"{{"forward_multimodal_content":{flag}}}"#))
}

pub(super) fn runtime_session_id() {
    assert_generated_session();
    assert_provided_session();
    assert_session_type_error();
}

fn assert_generated_session() {
    let context = messages(TEXT);
    let empty = params("{}");
    let mut request_headers = headers(&[]);
    let payload = transform_request(
        &context,
        &empty,
        &empty,
        &mut request_headers,
        generated_uuid(),
    )
    .unwrap();
    assert_eq!(header(&request_headers, SESSION_HEADER), Some(GENERATED));
    assert_eq!(payload, request_data(r#"{"prompt":"hi"}"#));
}

fn assert_provided_session() {
    assert_eq!(
        compute_session_id(&params(r#"{"runtimeSessionId":"short"}"#), generated_uuid()).unwrap(),
        "short",
        "a short caller id is sent unchanged"
    );
    assert_eq!(
        compute_session_id(&params(r#"{"runtimeSessionId":""}"#), generated_uuid()).unwrap(),
        GENERATED
    );
    assert_eq!(
        compute_session_id(&params(r#"{"runtimeSessionId":null}"#), generated_uuid()).unwrap(),
        GENERATED
    );
    let long = "a".repeat(40);
    assert_eq!(
        compute_session_id(
            &params(&format!(r#"{{"runtimeSessionId":"{long}"}}"#)),
            generated_uuid()
        )
        .unwrap(),
        long
    );
    let context = messages(TEXT);
    let mut caller_headers = headers(&[(SESSION_HEADER, Some("caller-session"))]);
    transform_request(
        &context,
        &params(r#"{"runtimeSessionId":"short"}"#),
        &params("{}"),
        &mut caller_headers,
        generated_uuid(),
    )
    .unwrap();
    assert_eq!(
        header(&caller_headers, SESSION_HEADER),
        Some("short"),
        "a supplied id overwrites the caller header"
    );
}

fn assert_session_type_error() {
    assert!(matches!(
        compute_session_id(&params(r#"{"runtimeSessionId":9}"#), generated_uuid()),
        Err(AgentCoreRequestError::WrongType {
            field: "runtimeSessionId"
        })
    ));
}

pub(super) fn runtime_user_id() {
    assert_provided_user();
    assert_caller_user();
    assert_user_direct();
}

fn assert_provided_user() {
    let mut request_headers = headers(&[(USER_HEADER, Some("caller-user"))]);
    transform_request(
        &messages(TEXT),
        &params(r#"{"runtimeUserId":"provided"}"#),
        &params("{}"),
        &mut request_headers,
        generated_uuid(),
    )
    .unwrap();
    assert_eq!(header(&request_headers, USER_HEADER), Some("provided"));
}

fn assert_caller_user() {
    assert_caller_user_case("{}");
    assert_caller_user_case(r#"{"runtimeUserId":null}"#);
    assert_caller_user_case(r#"{"runtimeUserId":""}"#);
}

fn assert_caller_user_case(optional: &str) {
    let mut request_headers = headers(&[(USER_HEADER, Some("caller-user"))]);
    let payload = transform_request(
        &messages(TEXT),
        &params(optional),
        &params("{}"),
        &mut request_headers,
        generated_uuid(),
    )
    .unwrap();
    assert_eq!(header(&request_headers, USER_HEADER), Some("caller-user"));
    assert_eq!(payload, request_data(r#"{"prompt":"hi"}"#));
    assert_eq!(header(&request_headers, SESSION_HEADER), Some(GENERATED));
}

fn assert_user_direct() {
    assert_eq!(
        compute_user_id(&params(r#"{"runtimeUserId":""}"#)).unwrap(),
        Some(String::new())
    );
    assert_eq!(compute_user_id(&params("{}")).unwrap(), None);
    assert!(matches!(
        compute_user_id(&params(r#"{"runtimeUserId":true}"#)),
        Err(AgentCoreRequestError::WrongType {
            field: "runtimeUserId"
        })
    ));
}

pub(super) fn multimodal_forwarding() {
    assert_forwarding_off_by_default();
    assert_string_policy();
    assert_other_value_truthiness();
    assert_source_precedence();
    assert_content_shape();
}

fn assert_forwarding_off_by_default() {
    assert_eq!(
        forwarded(&params("{}"), &params("{}")),
        request_data(r#"{"prompt":"summarize"}"#)
    );
    assert_eq!(
        forwarded(&flag_value("true"), &params("{}")),
        request_data(CONTENT)
    );
    assert_eq!(
        forwarded(&params("{}"), &flag_value("true")),
        request_data(CONTENT),
        "litellm_params is consulted after optional_params"
    );
}

fn assert_string_policy() {
    assert!(forwarded(&flag_value(r#""true""#), &params("{}")).contains_key("content"));
    assert!(forwarded(&flag_value(r#"" TRUE ""#), &params("{}")).contains_key("content"));
    assert!(forwarded(&flag_value(r#""1""#), &params("{}")).contains_key("content"));
    assert!(forwarded(&flag_value(r#""yes""#), &params("{}")).contains_key("content"));
    assert!(forwarded(&flag_value(r#""on""#), &params("{}")).contains_key("content"));
    assert_eq!(
        forwarded(&flag_value(r#""false""#), &params("{}")),
        request_data(r#"{"prompt":"summarize"}"#)
    );
    assert_eq!(
        forwarded(&flag_value(r#""off""#), &params("{}")),
        request_data(r#"{"prompt":"summarize"}"#)
    );
    assert_eq!(
        forwarded(&flag_value(r#""garbage""#), &params("{}")),
        request_data(r#"{"prompt":"summarize"}"#)
    );
}

fn assert_other_value_truthiness() {
    assert!(forwarded(&flag_value("2"), &params("{}")).contains_key("content"));
    assert!(forwarded(&flag_value("[1]"), &params("{}")).contains_key("content"));
    assert_eq!(
        forwarded(&flag_value("0"), &params("{}")),
        request_data(r#"{"prompt":"summarize"}"#)
    );
    assert_eq!(
        forwarded(&flag_value("[]"), &params("{}")),
        request_data(r#"{"prompt":"summarize"}"#)
    );
    assert_eq!(
        forwarded(&flag_value("{}"), &params("{}")),
        request_data(r#"{"prompt":"summarize"}"#)
    );
    assert_eq!(
        forwarded(&flag_value("false"), &params("{}")),
        request_data(r#"{"prompt":"summarize"}"#)
    );
}

fn assert_source_precedence() {
    assert_eq!(
        forwarded(&flag_value("false"), &flag_value("true")),
        request_data(r#"{"prompt":"summarize"}"#),
        "the first supplied value wins"
    );
    assert!(
        forwarded(&flag_value("null"), &flag_value("true")).contains_key("content"),
        "a null value defers to the next mapping"
    );
}

fn assert_content_shape() {
    assert_no_content_lists();
    assert_forwarded_lists();
}

fn assert_no_content_lists() {
    assert_eq!(
        payload_for(
            r#"[{"role":"user","content":[{"type":"text","text":"hello"}]}]"#,
            &flag_value("true"),
            &params("{}")
        )
        .unwrap(),
        request_data(r#"{"prompt":"hello"}"#),
        "a text-only list adds no content"
    );
    assert_eq!(
        payload_for(
            r#"[{"role":"user","content":[]}]"#,
            &flag_value("true"),
            &params("{}")
        )
        .unwrap(),
        request_data(r#"{"prompt":""}"#),
        "an empty list adds no content"
    );
    assert_eq!(
        payload_for(
            r#"[{"role":"user","content":[{"text":"hello"}]}]"#,
            &flag_value("true"),
            &params("{}")
        )
        .unwrap(),
        request_data(r#"{"prompt":"hello"}"#),
        "a block without a type adds no content"
    );
}

fn assert_forwarded_lists() {
    assert_unknown_block_forwarded();
    assert_last_message_only();
    assert_full_list_copied();
    assert_empty_messages_fail();
}

fn assert_unknown_block_forwarded() {
    assert_eq!(
        payload_for(
            r#"[{"role":"user","content":[{"type":"text","text":"transcribe"},{"type":"input_audio","input_audio":{"data":"U29tZUF1ZGlv","format":"wav"}}]}]"#,
            &flag_value("true"),
            &params("{}")
        )
        .unwrap(),
        request_data(
            r#"{"prompt":"transcribe","content":[{"type":"text","text":"transcribe"},{"type":"input_audio","input_audio":{"data":"U29tZUF1ZGlv","format":"wav"}}]}"#
        ),
        "unknown block types are forwarded as sent"
    );
}

fn assert_last_message_only() {
    assert_eq!(
        payload_for(
            r#"[{"role":"user","content":[{"type":"text","text":"old"},{"type":"image_url","image_url":{"url":"https://example.test/old.png"}}]},{"role":"assistant","content":"ok"},{"role":"user","content":"follow-up"}]"#,
            &flag_value("true"),
            &params("{}")
        )
        .unwrap(),
        request_data(r#"{"prompt":"follow-up"}"#),
        "only the last message decides"
    );
}

fn assert_full_list_copied() {
    assert_eq!(
        payload_for(
            r#"[{"role":"user","content":[{"type":"text","text":"first. "},{"type":"file","file":{"filename":"r.pdf","file_data":"data:application/pdf;base64,JVBERi0="}},{"type":"text","text":"second."},{"type":"image_url","image_url":{"url":"data:image/png;base64,iVBORw0KGgo="}}]}]"#,
            &flag_value("true"),
            &params("{}")
        )
        .unwrap(),
        request_data(
            r#"{"prompt":"first. second.","content":[{"type":"text","text":"first. "},{"type":"file","file":{"filename":"r.pdf","file_data":"data:application/pdf;base64,JVBERi0="}},{"type":"text","text":"second."},{"type":"image_url","image_url":{"url":"data:image/png;base64,iVBORw0KGgo="}}]}"#
        )
    );
}

fn assert_empty_messages_fail() {
    assert!(matches!(
        transform_request(
            &[],
            &params("{}"),
            &params("{}"),
            &mut headers(&[]),
            generated_uuid()
        ),
        Err(AgentCoreRequestError::EmptyMessages)
    ));
}
