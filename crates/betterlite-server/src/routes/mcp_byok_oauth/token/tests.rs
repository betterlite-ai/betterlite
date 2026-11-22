#![cfg(test)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use betterlite_mcp::byok_token_exchange::ports::AuthCodeRecord;
use serde::Deserialize;

use super::super::tests::{
    Built, FORM_TYPE, FakeSink, SinkCall, TestApp, VERIFIER, header, post_form, record, send, text,
};

const TOKEN_URI: &str = "/v1/mcp/oauth/token";

#[derive(Debug, Deserialize)]
struct GrantBody {
    access_token: String,
    token_type: String,
    expires_in: u32,
}

fn valid_body() -> String {
    format!("grant_type=authorization_code&code=auth-code-1&code_verifier={VERIFIER}")
}

fn post_with(content_type: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(TOKEN_URI)
        .header("content-type", content_type)
        .body(Body::from(body.to_owned()))
        .unwrap()
}

async fn exchange(app: TestApp, body: &str) -> (StatusCode, Response) {
    let Built { router, .. } = app.build();
    let response = send(router, post_form(TOKEN_URI, body)).await;
    (response.status(), response)
}

#[tokio::test]
async fn a_valid_code_returns_a_bearer_grant_with_no_cache_headers() {
    let (status, response) = exchange(TestApp::with_record(), &valid_body()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header(&response, "cache-control"), Some("no-store"));
    assert_eq!(header(&response, "pragma"), Some("no-cache"));
    let body = text(response).await;
    let grant = serde_json::from_str::<GrantBody>(&body).unwrap();
    assert_eq!(grant.token_type, "bearer");
    assert_eq!(grant.expires_in, 3600);
    assert!(!grant.access_token.is_empty());
}

#[tokio::test]
async fn a_valid_code_removes_the_code_and_stores_the_credential() {
    let sink = FakeSink::default();
    let mut app = TestApp::with_record();
    app.credentials = Some(sink.clone());
    let Built { router, codes } = app.build();
    let response = send(router, post_form(TOKEN_URI, &valid_body())).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(codes.removed());
    assert_eq!(sink.calls(), [SinkCall::Store, SinkCall::Invalidate]);
}

#[tokio::test]
async fn an_unsupported_grant_type_answers_its_error_code() {
    let body = format!("grant_type=refresh_token&code=auth-code-1&code_verifier={VERIFIER}");
    let (status, response) = exchange(TestApp::with_record(), &body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(header(&response, "cache-control"), Some("no-store"));
    assert_eq!(header(&response, "pragma"), Some("no-cache"));
    assert_eq!(
        text(response).await,
        r#"{"error":"unsupported_grant_type"}"#
    );
}

#[tokio::test]
async fn an_unknown_code_answers_invalid_grant() {
    let mut app = TestApp::with_record();
    app.record = None;
    let (status, response) = exchange(app, &valid_body()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(text(response).await, r#"{"error":"invalid_grant"}"#);
}

#[tokio::test]
async fn an_expired_code_answers_invalid_grant_and_is_removed() {
    let mut app = TestApp::with_record();
    app.record = Some(AuthCodeRecord {
        expires_at: 1.0,
        ..record()
    });
    let Built { router, codes } = app.build();
    let response = send(router, post_form(TOKEN_URI, &valid_body())).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(text(response).await, r#"{"error":"invalid_grant"}"#);
    assert!(codes.removed());
}

#[tokio::test]
async fn a_wrong_verifier_answers_invalid_grant_and_keeps_the_code() {
    let body = "grant_type=authorization_code&code=auth-code-1&code_verifier=wrong";
    let Built { router, codes } = TestApp::with_record().build();
    let response = send(router, post_form(TOKEN_URI, body)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(text(response).await, r#"{"error":"invalid_grant"}"#);
    assert!(!codes.removed());
}

#[tokio::test]
async fn a_failed_store_answers_server_error_after_removing_the_code() {
    let mut app = TestApp::with_record();
    app.credentials = Some(FakeSink::failing_store());
    let Built { router, codes } = app.build();
    let response = send(router, post_form(TOKEN_URI, &valid_body())).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(header(&response, "cache-control"), Some("no-store"));
    assert_eq!(text(response).await, r#"{"error":"server_error"}"#);
    assert!(codes.removed());
}

#[tokio::test]
async fn a_failed_invalidation_answers_server_error() {
    let sink = FakeSink::failing_invalidate();
    let mut app = TestApp::with_record();
    app.credentials = Some(sink.clone());
    let (status, response) = exchange(app, &valid_body()).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(text(response).await, r#"{"error":"server_error"}"#);
    assert_eq!(sink.calls(), [SinkCall::Store, SinkCall::Invalidate]);
}

#[tokio::test]
async fn a_missing_database_still_returns_a_grant() {
    let mut app = TestApp::with_record();
    app.credentials = None;
    let (status, _response) = exchange(app, &valid_body()).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_missing_form_body_lists_every_required_field() {
    let (status, response) = exchange(TestApp::with_record(), "").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(header(&response, "cache-control"), None);
    assert_eq!(
        text(response).await,
        concat!(
            r#"{"detail":[{"type":"missing","loc":["body","grant_type"],"msg":"Field required","input":null},"#,
            r#"{"type":"missing","loc":["body","code"],"msg":"Field required","input":null},"#,
            r#"{"type":"missing","loc":["body","code_verifier"],"msg":"Field required","input":null}]}"#,
        )
    );
}

#[tokio::test]
async fn only_the_missing_required_field_is_listed() {
    let body = "grant_type=authorization_code&code=auth-code-1";
    let (status, response) = exchange(TestApp::with_record(), body).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        text(response).await,
        concat!(
            r#"{"detail":[{"type":"missing","loc":["body","code_verifier"],"#,
            r#""msg":"Field required","input":null}]}"#,
        )
    );
}

#[tokio::test]
async fn an_empty_required_field_counts_as_missing() {
    let body = format!("grant_type=authorization_code&code=&code_verifier={VERIFIER}");
    let (status, response) = exchange(TestApp::with_record(), body.as_str()).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        text(response)
            .await
            .contains(r#"{"type":"missing","loc":["body","code"]"#)
    );
}

#[tokio::test]
async fn a_wrong_content_type_is_treated_as_an_empty_form() {
    let Built { router, .. } = TestApp::with_record().build();
    let response = send(router, post_with("text/plain", &valid_body())).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn an_uppercase_content_type_is_accepted() {
    let Built { router, .. } = TestApp::with_record().build();
    let response = send(
        router,
        post_with("APPLICATION/X-WWW-FORM-URLENCODED", &valid_body()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_content_type_with_a_charset_is_accepted() {
    let Built { router, .. } = TestApp::with_record().build();
    let response = send(
        router,
        post_with(
            "application/x-www-form-urlencoded; charset=UTF-8",
            &valid_body(),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_multipart_body_is_treated_as_an_empty_form() {
    let Built { router, .. } = TestApp::with_record().build();
    let request = post_with("multipart/form-data; boundary=frame", &valid_body());
    let response = send(router, request).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn a_duplicated_field_uses_the_last_value() {
    let body = format!(
        "grant_type=authorization_code&code=wrong&code=auth-code-1&code_verifier={VERIFIER}"
    );
    let (status, _response) = exchange(TestApp::with_record(), &body).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn an_empty_last_duplicate_counts_as_missing() {
    let body =
        format!("grant_type=authorization_code&code=auth-code-1&code=&code_verifier={VERIFIER}");
    let (status, response) = exchange(TestApp::with_record(), &body).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        text(response)
            .await
            .contains(r#"{"type":"missing","loc":["body","code"]"#)
    );
}

#[tokio::test]
async fn unknown_fields_are_ignored() {
    let body = format!("{}&unrelated=value", valid_body());
    let (status, _response) = exchange(TestApp::with_record(), &body).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn more_than_a_thousand_fields_answer_the_starlette_error() {
    let extras: Vec<String> = (0_usize..998).map(|index| format!("x{index}=y")).collect();
    let body = format!("{}&{}", valid_body(), extras.join("&"));
    let (status, response) = exchange(TestApp::with_record(), &body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        text(response).await,
        r#"{"detail":"Too many fields. Maximum number of fields is 1000."}"#
    );
}

#[tokio::test]
async fn exactly_a_thousand_fields_is_accepted() {
    let extras: Vec<String> = (0_usize..997).map(|index| format!("x{index}=y")).collect();
    let body = format!("{}&{}", valid_body(), extras.join("&"));
    let (status, _response) = exchange(TestApp::with_record(), &body).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_body_over_one_megabyte_is_refused() {
    let body = format!("{}&padding={}", valid_body(), "a".repeat(0x0010_0001));
    let (status, response) = exchange(TestApp::with_record(), &body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        text(response).await,
        r#"{"detail":"There was an error parsing the body"}"#
    );
}

#[tokio::test]
async fn a_percent_encoded_unicode_verifier_is_decoded_as_utf8() {
    let mut app = TestApp::with_record();
    app.record = Some(AuthCodeRecord {
        code_challenge: "CRq1B87KKMcG1TcI6MGzKFE7NtDLjfpfofPinwjxMOs".to_owned(),
        ..record()
    });
    let body = "grant_type=authorization_code&code=auth-code-1&code_verifier=%C3%BC%C3%B1%C3%AF%C3%A7%C3%B8d%C3%A9";
    let (status, _response) = exchange(app, body).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_raw_utf8_verifier_is_read_as_latin1() {
    let mut body = b"grant_type=authorization_code&code=auth-code-1&code_verifier=".to_vec();
    body.extend_from_slice("\u{fc}\u{f1}\u{ef}\u{e7}\u{f8}d\u{e9}".as_bytes());
    let Built { router, .. } = TestApp::with_record().build();
    let request = Request::builder()
        .method("POST")
        .uri(TOKEN_URI)
        .header("content-type", FORM_TYPE)
        .body(Body::from(body))
        .unwrap();
    let response = send(router, request).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(text(response).await, r#"{"error":"invalid_grant"}"#);
}

#[tokio::test]
async fn a_body_over_the_segment_cap_answers_the_field_error() {
    let body = "&".repeat(1001);
    let (status, response) = exchange(TestApp::with_record(), &body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        text(response).await,
        r#"{"detail":"Too many fields. Maximum number of fields is 1000."}"#
    );
}
