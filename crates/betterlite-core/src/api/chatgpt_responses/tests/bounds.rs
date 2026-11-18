#![cfg(test)]

use std::str::from_utf8;

use betterlite_openai::error::OpenAiError;

use crate::api::api_boundaries::{ApiLimitError, ApiLimits};
use crate::api::chatgpt_responses::ChatGptResponsesError;
use crate::api::responses_sse_recovery::ResponsesSseRecoveryError;

use super::{Fixture, JSON_TYPE, SSE_TYPE, done, expect_error, headers, item, text, typed};

fn limits(max_event_bytes: usize, max_items: usize) -> ApiLimits {
    ApiLimits {
        max_json_bytes: 16 * 1024 * 1024,
        max_event_bytes,
        max_items,
    }
}

#[test]
fn sse_rejects_an_event_over_the_boundary() {
    let body = vec![done()];
    let error = expect_error(Fixture::new().transform(
        200,
        &headers(SSE_TYPE),
        &body.join("\n"),
        &limits(8, 64 * 1024),
    ));
    assert_eq!(error.http_status(), Some(502));
    assert!(matches!(
        error,
        ChatGptResponsesError::Recovery {
            source: ResponsesSseRecoveryError::Limit(ApiLimitError::EventTooLarge { .. })
        }
    ));
}

#[test]
fn sse_rejects_more_items_than_the_boundary() {
    let body = vec![item(0, "zero"), item(1, "one"), done()];
    let error = expect_error(Fixture::new().transform(
        200,
        &headers(SSE_TYPE),
        &body.join("\n"),
        &limits(1024 * 1024, 1),
    ));
    assert_eq!(error.http_status(), Some(400));
    assert!(matches!(
        error,
        ChatGptResponsesError::Recovery {
            source: ResponsesSseRecoveryError::Limit(ApiLimitError::TooManyItems { .. })
        }
    ));
}

#[test]
fn a_body_over_the_total_boundary_is_rejected() {
    let budget = ApiLimits {
        max_json_bytes: 32,
        max_event_bytes: 1024 * 1024,
        max_items: 64 * 1024,
    };
    let streamed = vec![text(0, "first"), text(1, "second"), done()].join("\n");
    let streamed_error =
        expect_error(Fixture::new().transform(200, &headers(SSE_TYPE), &streamed, &budget));
    assert_eq!(streamed_error.http_status(), Some(502));
    assert!(matches!(
        streamed_error,
        ChatGptResponsesError::ReplyTooLarge { observed, limit }
            if observed == streamed.len() && limit == 32
    ));
    let json =
        String::from(r#"{"id":"resp_1","created_at":1700000000,"object":"response","output":[]}"#);
    let json_error =
        expect_error(Fixture::new().transform(200, &headers(JSON_TYPE), &json, &budget));
    assert_eq!(json_error.http_status(), Some(502));
    assert!(matches!(
        json_error,
        ChatGptResponsesError::ReplyTooLarge { observed, limit }
            if observed == json.len() && limit == 32
    ));
}

#[test]
fn a_body_exactly_at_the_total_boundary_is_accepted() {
    let streamed = vec![text(0, "first"), done()].join("\n");
    let budget = ApiLimits {
        max_json_bytes: streamed.len(),
        max_event_bytes: 1024 * 1024,
        max_items: 64 * 1024,
    };
    let reply = Fixture::new()
        .transform(200, &headers(SSE_TYPE), &streamed, &budget)
        .expect("a body exactly at the boundary is accepted");
    assert_eq!(typed(&reply).id, "resp_1");
}

#[test]
fn recovery_errors_without_a_status_stay_statusless() {
    let bytes = from_utf8(b"\xff").expect_err("the bytes are not UTF-8");
    let utf8_error = ChatGptResponsesError::Recovery {
        source: ResponsesSseRecoveryError::Utf8(bytes),
    };
    assert_eq!(utf8_error.http_status(), None);
    let encode = ChatGptResponsesError::Encode {
        source: serde_json::from_str::<i32>("x").expect_err("the value is not an integer"),
    };
    assert_eq!(encode.http_status(), None);
    let generic = ChatGptResponsesError::Transform(OpenAiError::MissingKey { key: "x" });
    assert_eq!(generic.http_status(), None);
}
