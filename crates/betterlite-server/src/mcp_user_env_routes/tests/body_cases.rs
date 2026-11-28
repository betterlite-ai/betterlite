use axum::http::StatusCode;

use super::support::{Harness, MEMBER, Reply, TestIssue, call, context, post_raw, server_uri};

const REQUIRED: &str = "Field required";
const JSON_DECODE_ERROR: &str = "JSON decode error";
const ATTRIBUTES_TYPE: &str = "Input should be a valid dictionary or object to extract fields from";
const DICT_TYPE: &str = "Input should be a valid dictionary";

fn issue(kind: &str, loc: &[&str], msg: &str) -> TestIssue {
    TestIssue {
        kind: kind.to_owned(),
        loc: loc.iter().map(|part| String::from(*part)).collect(),
        msg: msg.to_owned(),
    }
}

async fn post_body(content_type: Option<&str>, body: &[u8]) -> Reply {
    let harness = Harness::new();
    call(
        harness.router(),
        post_raw(
            &server_uri("srv-1"),
            Some(&context(MEMBER)),
            content_type,
            body,
        ),
    )
    .await
}

#[tokio::test]
async fn an_empty_body_is_missing() {
    let reply = post_body(Some("application/json"), b"").await;
    assert_eq!(reply.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.issues(), [issue("missing", &["body"], REQUIRED)]);
}

#[tokio::test]
async fn a_null_body_is_missing() {
    let reply = post_body(Some("application/json"), b"null").await;
    assert_eq!(reply.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.issues(), [issue("missing", &["body"], REQUIRED)]);
}

#[tokio::test]
async fn a_missing_values_key_is_missing() {
    let reply = post_body(Some("application/json"), b"{}").await;
    assert_eq!(reply.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        reply.issues(),
        [issue("missing", &["body", "values"], REQUIRED)]
    );
}

#[tokio::test]
async fn a_null_values_is_a_dict_type() {
    let reply = post_body(Some("application/json"), br#"{"values":null}"#).await;
    assert_eq!(reply.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        reply.issues(),
        [issue("dict_type", &["body", "values"], DICT_TYPE)]
    );
}

#[tokio::test]
async fn a_non_object_values_is_a_dict_type() {
    let reply = post_body(Some("application/json"), br#"{"values":[]}"#).await;
    assert_eq!(reply.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        reply.issues(),
        [issue("dict_type", &["body", "values"], DICT_TYPE)]
    );
}

#[tokio::test]
async fn a_non_string_value_is_a_string_type() {
    let reply = post_body(Some("application/json"), br#"{"values":{"TOKEN":7}}"#).await;
    assert_eq!(reply.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        reply.issues(),
        [issue(
            "string_type",
            &["body", "values", "TOKEN"],
            "Input should be a valid string"
        )]
    );
}

#[tokio::test]
async fn every_bad_value_is_reported_in_input_order() {
    let reply = post_body(
        Some("application/json"),
        br#"{"values":{"FIRST":1,"SECOND":true}}"#,
    )
    .await;
    assert_eq!(reply.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        reply.issues(),
        [
            issue(
                "string_type",
                &["body", "values", "FIRST"],
                "Input should be a valid string"
            ),
            issue(
                "string_type",
                &["body", "values", "SECOND"],
                "Input should be a valid string"
            )
        ]
    );
}

#[tokio::test]
async fn a_validation_issue_carries_no_echoed_input() {
    let reply = post_body(Some("application/json"), br#"{"values":{"TOKEN":7}}"#).await;
    let text = reply.text();
    assert!(!text.contains("\"input\""));
    assert!(!text.contains("\"ctx\""));
    assert!(!text.contains("\"url\""));
}

#[tokio::test]
async fn a_top_level_list_is_not_a_model() {
    let reply = post_body(Some("application/json"), b"[1]").await;
    assert_eq!(reply.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        reply.issues(),
        [issue("model_attributes_type", &["body"], ATTRIBUTES_TYPE)]
    );
}

#[tokio::test]
async fn a_scalar_body_is_not_a_model() {
    let reply = post_body(Some("application/json"), b"5").await;
    assert_eq!(reply.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        reply.issues(),
        [issue("model_attributes_type", &["body"], ATTRIBUTES_TYPE)]
    );
}

#[tokio::test]
async fn malformed_json_is_a_json_invalid() {
    let reply = post_body(Some("application/json"), b"{").await;
    assert_eq!(reply.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        reply.issues(),
        [issue("json_invalid", &["body"], JSON_DECODE_ERROR)]
    );
}

#[tokio::test]
async fn a_non_json_media_type_does_not_parse_json() {
    let reply = post_body(Some("text/plain"), br#"{"values":{"TOKEN":"x"}}"#).await;
    assert_eq!(reply.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        reply.issues(),
        [issue("model_attributes_type", &["body"], ATTRIBUTES_TYPE)]
    );
}

#[tokio::test]
async fn invalid_utf8_is_the_http_400() {
    let reply = post_body(Some("application/json"), b"\xff\xfe").await;
    assert_eq!(reply.status(), StatusCode::BAD_REQUEST);
    assert_eq!(reply.text_detail(), "There was an error parsing the body");
}

#[tokio::test]
async fn an_empty_content_type_still_reads_json() {
    let reply = post_body(Some(""), br#"{"values":{"TOKEN":"secret"}}"#).await;
    assert_eq!(reply.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_charset_parameter_still_reads_json() {
    let reply = post_body(
        Some("application/json; charset=utf-8"),
        br#"{"values":{"TOKEN":"secret"}}"#,
    )
    .await;
    assert_eq!(reply.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_vendor_json_subtype_still_reads_json() {
    let reply = post_body(
        Some("application/vnd.betterlite+json"),
        br#"{"values":{"TOKEN":"secret"}}"#,
    )
    .await;
    assert_eq!(reply.status(), StatusCode::OK);
}

#[tokio::test]
async fn no_content_type_still_reads_json() {
    let reply = post_body(None, br#"{"values":{"TOKEN":"secret"}}"#).await;
    assert_eq!(reply.status(), StatusCode::OK);
}

#[tokio::test]
async fn unknown_top_level_fields_are_ignored() {
    let reply = post_body(
        Some("application/json"),
        br#"{"values":{"TOKEN":"secret"},"extra":true}"#,
    )
    .await;
    assert_eq!(reply.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_body_error_wins_over_the_database_check() {
    let harness = Harness::new().without_database();
    let reply = call(
        harness.router(),
        post_raw(
            &server_uri("srv-1"),
            Some(&context(MEMBER)),
            Some("application/json"),
            b"{",
        ),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(harness.journal().is_empty());
}

#[tokio::test]
async fn a_refused_body_never_reaches_the_store() {
    let harness = Harness::new();
    let reply = call(
        harness.router(),
        post_raw(
            &server_uri("srv-1"),
            Some(&context(MEMBER)),
            Some("text/plain"),
            br#"{"values":{"TOKEN":"secret"}}"#,
        ),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(harness.journal().is_empty());
}
