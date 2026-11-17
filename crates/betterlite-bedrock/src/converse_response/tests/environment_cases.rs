#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::bedrock_settings::HeaderError;
use crate::converse_response::ReplyFieldError;
use crate::converse_response::environment::{
    get_error_class, should_fake_stream, validate_environment,
};
use crate::converse_response::message::MessageError;
use crate::converse_response::transform::TransformError;
use crate::request_policy::{BedrockRequestPolicy, Params, RequestValue};

use super::support::{convert, convert_bytes, fixture, json};

type FakeStreamRow = (Option<&'static str>, Option<bool>, Option<bool>, bool);

pub(super) fn fake_stream_capability() {
    let cases: [FakeStreamRow; 11] = [
        (None, None, None, false),
        (Some("ai21.j2-mid"), Some(true), None, true),
        (Some("ai21.j2-mid"), Some(false), None, false),
        (Some("ai21.j2-mid"), None, None, false),
        (Some("anthropic.claude-3-sonnet"), Some(true), None, false),
        (
            Some("anthropic.claude-3-sonnet"),
            Some(true),
            Some(false),
            false,
        ),
        (Some("anthropic.claude-3-sonnet"), None, Some(true), true),
        (Some("AI21.J2"), Some(true), None, false),
        (Some(""), Some(true), None, false),
        (Some("ai21.j2-mid"), Some(false), Some(true), true),
        (Some("anthropic.claude-3-sonnet"), Some(false), None, false),
    ];
    for (model, stream, fake_stream, expected) in cases {
        assert_eq!(
            should_fake_stream(model, stream, fake_stream),
            expected,
            "model {model:?} stream {stream:?} fake_stream {fake_stream:?}"
        );
    }
}

pub(super) fn malformed_json() {
    an_unreadable_body_is_a_422();
    a_non_object_body_is_a_422();
    missing_output_members_keep_their_own_failure();
    a_message_without_content_keeps_its_own_failure();
    missing_later_members_keep_their_own_failure();
    a_malformed_usage_keeps_its_own_failure();
    a_non_list_content_is_not_a_block_list();
    a_reply_missing_usage_and_stop_reason_names_the_stop_reason();
    an_unreadable_body_is_logged_by_its_byte_count();
    a_422_with_non_ascii_headers_is_a_header_error();
    error_class_delegates_the_supplied_fields();
}

fn an_unreadable_body_is_a_422() {
    let mut fixture = fixture();
    fixture.headers = vec![
        (String::from("x-amzn-requestid"), String::from("req-1")),
        (
            String::from("x-amzn-errortype"),
            String::from("ValidationException"),
        ),
    ];
    let run = convert(&fixture, "{not json");
    assert_eq!(run.status(), Some(422));
    let TransformError::Conversion { error, .. } = run.failure() else {
        panic!("an unreadable body is a conversion failure");
    };
    assert_eq!(error.error.status_code, 422);
    assert!(!error.error.status_code_is_synthesized);
    assert_eq!(error.response_headers, fixture.headers);
    assert!(
        error
            .error
            .message
            .starts_with("Error converting to valid response block=")
    );
    assert!(
        error.error.message.ends_with(
            ". File an issue if litellm error - https://github.com/BerriAI/litellm/issues"
        )
    );
}

fn a_non_object_body_is_a_422() {
    let mut fixture = fixture();
    fixture.headers = vec![(String::from("x-amzn-requestid"), String::from("req-2"))];
    let run = convert(&fixture, "[]");
    let TransformError::Conversion { error, .. } = run.failure() else {
        panic!("a list body is a conversion failure");
    };
    assert_eq!(error.error.status_code, 422);
    assert_eq!(
        error.error.message,
        "Error converting to valid response block=the body is not a JSON object. File an issue if litellm error - https://github.com/BerriAI/litellm/issues"
    );
    assert_eq!(error.response_headers, fixture.headers);

    let bare = run_scalar_body();
    let TransformError::Conversion {
        error: scalar_error,
        ..
    } = bare.failure()
    else {
        panic!("a scalar body is a conversion failure");
    };
    assert_eq!(scalar_error.error.status_code, 422);
    assert!(scalar_error.response_headers.is_empty());
}

fn run_scalar_body() -> super::support::Run {
    let mut fixture = fixture();
    fixture.headers = Vec::new();
    convert(&fixture, "42")
}

fn missing_output_members_keep_their_own_failure() {
    let fixture = fixture();
    let missing_output = convert(&fixture, "{}");
    assert_eq!(missing_output.status(), None);
    assert!(matches!(
        missing_output.failure(),
        TransformError::Message(MessageError::Reply(ReplyFieldError::Missing {
            path: "output"
        }))
    ));

    let missing_message = convert(&fixture, r#"{"output": {}}"#);
    assert_eq!(missing_message.status(), None);
    assert!(matches!(
        missing_message.failure(),
        TransformError::Message(MessageError::Reply(ReplyFieldError::Missing {
            path: "output.message"
        }))
    ));

    let not_an_output = convert(
        &fixture,
        r#"{"output": [], "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#,
    );
    assert!(matches!(
        not_an_output.failure(),
        TransformError::Message(MessageError::Reply(ReplyFieldError::NotA {
            path: "output",
            expected: "an object"
        }))
    ));

    let not_a_message = convert(
        &fixture,
        r#"{"output": {"message": "just text"}, "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#,
    );
    assert!(matches!(
        not_a_message.failure(),
        TransformError::Message(MessageError::Reply(ReplyFieldError::NotA {
            path: "output.message",
            expected: "an object"
        }))
    ));
}

fn missing_later_members_keep_their_own_failure() {
    let fixture = fixture();
    let missing_usage = convert(
        &fixture,
        r#"{"output": {"message": {"role": "assistant", "content": []}}, "stopReason": "end_turn"}"#,
    );
    assert_eq!(missing_usage.status(), None);
    assert!(matches!(
        missing_usage.failure(),
        TransformError::Reply(ReplyFieldError::Missing { path: "usage" })
    ));

    let missing_stop_reason = convert(
        &fixture,
        r#"{"output": {"message": {"role": "assistant", "content": []}}, "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#,
    );
    assert_eq!(missing_stop_reason.status(), None);
    assert!(matches!(
        missing_stop_reason.failure(),
        TransformError::Message(MessageError::Reply(ReplyFieldError::Missing {
            path: "stopReason"
        }))
    ));
}

fn a_message_without_content_keeps_its_own_failure() {
    let body = r#"{"output": {"message": {}}, "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#;
    let run = convert(&fixture(), body);
    assert_eq!(run.status(), None);
    assert!(matches!(
        run.failure(),
        TransformError::Message(MessageError::Reply(ReplyFieldError::Missing {
            path: "output.message.content"
        }))
    ));
}

fn a_malformed_usage_keeps_its_own_failure() {
    let body = r#"{"output": {"message": {"content": []}}, "stopReason": "end_turn", "usage": 42}"#;
    let run = convert(&fixture(), body);
    assert_eq!(run.status(), None);
    assert_eq!(
        run.detail("original_response"),
        &Json::Text(body.to_owned())
    );
    assert!(matches!(run.failure(), TransformError::TokenUsage(_)));
}

// LiteLLM iterates a string content character by character and answers with empty content; a typed error names the field instead.
fn a_non_list_content_is_not_a_block_list() {
    for body in [
        r#"{"output": {"message": {"content": "just text"}}, "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#,
        r#"{"output": {"message": {"content": 42}}, "stopReason": "end_turn", "usage": {"inputTokens": 1, "outputTokens": 1, "totalTokens": 2}}"#,
    ] {
        let run = convert(&fixture(), body);
        assert_eq!(run.status(), None);
        assert!(matches!(
            run.failure(),
            TransformError::Message(MessageError::Reply(ReplyFieldError::NotA {
                path: "output.message.content",
                expected: "a list of content blocks"
            }))
        ));
    }
}

// The conversion reads stopReason before usage, so a reply missing both names stopReason; LiteLLM reads usage first.
fn a_reply_missing_usage_and_stop_reason_names_the_stop_reason() {
    let run = convert(&fixture(), r#"{"output": {"message": {"content": []}}}"#);
    assert_eq!(run.status(), None);
    assert!(matches!(
        run.failure(),
        TransformError::Message(MessageError::Reply(ReplyFieldError::Missing {
            path: "stopReason"
        }))
    ));
}

// LiteLLM logs httpx's replacement-character text; betterlite names the body by its byte count.
fn an_unreadable_body_is_logged_by_its_byte_count() {
    let body: &[u8] = b"{\"output\": \"\xff\"}";
    let run = convert_bytes(&fixture(), body);
    assert_eq!(
        run.detail("original_response"),
        &json(&format!("\"{} bytes that are not UTF-8\"", body.len()))
    );
    assert_eq!(run.status(), Some(422));
}

// The conversion refuses an upstream header it cannot encode as ASCII, so the 422 becomes a header error.
fn a_422_with_non_ascii_headers_is_a_header_error() {
    let mut fixture = fixture();
    fixture.headers = vec![(String::from("x-amzn-requestid"), String::from("req-\u{e9}"))];
    let run = convert(&fixture, "{not json");
    assert!(matches!(
        run.failure(),
        TransformError::Header(HeaderError::NotAscii { name }) if name.as_str() == "x-amzn-requestid"
    ));
}

fn error_class_delegates_the_supplied_fields() {
    let headers = vec![(String::from("x-amzn-requestid"), String::from("req-1"))];
    let error = get_error_class("conversion failed", 422, &headers).expect("the error builds");
    assert_eq!(error.error.status_code, 422);
    assert_eq!(error.error.message, "conversion failed");
    assert!(!error.error.status_code_is_synthesized);
    assert_eq!(error.response_headers, headers);
    assert_eq!(error.error.request.method, "POST");
    assert_eq!(error.error.request.url, "https://docs.litellm.ai/docs");
    assert_eq!(error.error.response.status_code, 422);

    let throttled_headers = vec![
        (String::from("Retry-After"), String::from("3")),
        (String::from("x-amzn-requestid"), String::from("req-2")),
    ];
    let throttled =
        get_error_class("throttled", 429, &throttled_headers).expect("the error builds");
    assert_eq!(throttled.error.status_code, 429);
    assert_eq!(throttled.response_headers, throttled_headers);
}

pub(super) fn auth_and_metadata_headers() {
    let empty = params(Vec::new());
    api_key_writes_the_bearer_header(&empty);
    the_metadata_owner_writes_and_evicts_the_header(&empty);
    an_unowned_metadata_header_is_kept(&empty);
    spend_logs_metadata_fills_the_header();
    identity_and_key_fill_their_headers();
    both_metadata_sources_merge_into_one_header();
    an_unforwardable_identity_leaves_no_header();
}

fn headers(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn params(entries: Vec<(&str, RequestValue)>) -> Params {
    entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

fn mapping(entries: Vec<(&str, RequestValue)>) -> RequestValue {
    RequestValue::Mapping(params(entries))
}

fn text(value: &str) -> RequestValue {
    RequestValue::Text(value.to_owned())
}

fn policy(fields: &[&str]) -> BedrockRequestPolicy {
    BedrockRequestPolicy {
        fields: fields.iter().map(|field| (*field).to_owned()).collect(),
    }
}

fn api_key_writes_the_bearer_header(empty: &Params) {
    let base = headers(&[("X-Caller", "kept")]);
    let untouched = validate_environment(&base, None, None, empty).expect("the headers merge");
    assert_eq!(untouched, base);

    let keyed = validate_environment(&base, Some(&SecretString::from("bedrock-key")), None, empty)
        .expect("the headers merge");
    assert_eq!(
        keyed,
        headers(&[
            ("X-Caller", "kept"),
            ("Authorization", "Bearer bedrock-key")
        ])
    );

    let blank = validate_environment(
        &headers(&[("Authorization", "caller-token")]),
        Some(&SecretString::from("")),
        None,
        empty,
    )
    .expect("the headers merge");
    assert_eq!(blank, headers(&[("Authorization", "caller-token")]));

    let lower = validate_environment(
        &headers(&[("authorization", "caller-token")]),
        Some(&SecretString::from("bedrock-key")),
        None,
        empty,
    )
    .expect("the headers merge");
    assert_eq!(
        lower,
        headers(&[
            ("authorization", "caller-token"),
            ("Authorization", "Bearer bedrock-key")
        ])
    );
}

fn the_metadata_owner_writes_and_evicts_the_header(empty: &Params) {
    let identity = policy(&["user_api_key_team_id"]);
    let metadata = params(vec![(
        "metadata",
        mapping(vec![("user_api_key_team_id", text("team-7"))]),
    )]);
    let merged = validate_environment(
        &headers(&[("X-Caller", "kept")]),
        None,
        Some(&identity),
        &metadata,
    )
    .expect("the headers merge");
    assert_eq!(
        merged,
        headers(&[
            ("X-Caller", "kept"),
            (
                "X-Amzn-Bedrock-Request-Metadata",
                r#"{"user_api_key_team_id":"team-7"}"#
            )
        ])
    );

    let evicted = validate_environment(
        &headers(&[
            ("x-amzn-bedrock-request-metadata", "caller"),
            ("X-Caller", "kept"),
        ]),
        None,
        Some(&identity),
        empty,
    )
    .expect("the headers merge");
    assert_eq!(evicted, headers(&[("X-Caller", "kept")]));
}

fn an_unowned_metadata_header_is_kept(empty: &Params) {
    let unowned = validate_environment(
        &headers(&[("x-amzn-bedrock-request-metadata", "caller")]),
        None,
        None,
        empty,
    )
    .expect("the headers merge");
    assert_eq!(
        unowned,
        headers(&[("x-amzn-bedrock-request-metadata", "caller")])
    );
}

fn spend_logs_metadata_fills_the_header() {
    let spend = policy(&["spend_logs_metadata"]);
    let spend_params = params(vec![(
        "metadata",
        mapping(vec![(
            "spend_logs_metadata",
            mapping(vec![("team", text("a")), ("user", text("b"))]),
        )]),
    )]);
    let spend_headers = validate_environment(&headers(&[]), None, Some(&spend), &spend_params)
        .expect("the headers merge");
    assert_eq!(
        spend_headers,
        headers(&[(
            "X-Amzn-Bedrock-Request-Metadata",
            r#"{"team":"a","user":"b"}"#
        )])
    );
}

fn identity_and_key_fill_their_headers() {
    let identity = policy(&["user_api_key_team_id"]);
    let litellm_params = params(vec![(
        "litellm_metadata",
        mapping(vec![("user_api_key_team_id", text("team-9"))]),
    )]);
    let identity_headers = validate_environment(
        &headers(&[]),
        Some(&SecretString::from("bedrock-key")),
        Some(&identity),
        &litellm_params,
    )
    .expect("the headers merge");
    assert_eq!(
        identity_headers,
        headers(&[
            ("Authorization", "Bearer bedrock-key"),
            (
                "X-Amzn-Bedrock-Request-Metadata",
                r#"{"user_api_key_team_id":"team-9"}"#
            )
        ])
    );
}

fn both_metadata_sources_merge_into_one_header() {
    let both = policy(&["user_api_key_team_id", "spend_logs_metadata"]);
    let mixed = params(vec![(
        "metadata",
        mapping(vec![
            ("user_api_key_team_id", text("team-7")),
            ("spend_logs_metadata", mapping(vec![("tag", text("a b"))])),
        ]),
    )]);
    let mixed_headers = validate_environment(
        &headers(&[]),
        Some(&SecretString::from("bedrock-key")),
        Some(&both),
        &mixed,
    )
    .expect("the headers merge");
    assert_eq!(
        mixed_headers,
        headers(&[
            ("Authorization", "Bearer bedrock-key"),
            (
                "X-Amzn-Bedrock-Request-Metadata",
                r#"{"user_api_key_team_id":"team-7","tag":"a b"}"#
            )
        ])
    );
}

fn an_unforwardable_identity_leaves_no_header() {
    let identity = policy(&["user_api_key_team_id"]);
    let invalid = params(vec![(
        "metadata",
        mapping(vec![("user_api_key_team_id", text("bad key!"))]),
    )]);
    let merged = validate_environment(
        &headers(&[("X-Amzn-Bedrock-Request-Metadata", "caller")]),
        None,
        Some(&identity),
        &invalid,
    )
    .expect("the headers merge");
    assert!(merged.is_empty());
}
