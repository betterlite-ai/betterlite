#![cfg(test)]

mod http;

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::RefreshTransport as _;
use super::{
    GrantAnswer, GrantPostTransport, NON_OBJECT_DETAIL, RefreshGrantPost, UNREACHABLE_DETAIL,
    map_answer,
};
use crate::mcp::outbound_credentials::sso_assertion_refresher::failure::RefreshFailureKind;

const TOKEN_ENDPOINT: &str = "https://idp.example/token";

#[derive(Clone)]
struct SentPost {
    url: String,
    form: BTreeMap<String, String>,
    headers: BTreeMap<String, String>,
}

struct ScriptedPost {
    answers: Mutex<VecDeque<GrantAnswer>>,
    calls: Mutex<Vec<SentPost>>,
}

#[async_trait]
impl RefreshGrantPost for ScriptedPost {
    async fn post_form(
        &self,
        url: &str,
        form: &BTreeMap<String, String>,
        headers: &BTreeMap<String, String>,
    ) -> GrantAnswer {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SentPost {
                url: url.to_owned(),
                form: form.clone(),
                headers: headers.clone(),
            });
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("the case scripts an answer")
    }
}

fn answering(answer: GrantAnswer) -> GrantPostTransport<ScriptedPost> {
    GrantPostTransport {
        post: ScriptedPost {
            answers: Mutex::new(VecDeque::from([answer])),
            calls: Mutex::new(Vec::new()),
        },
    }
}

fn form() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("grant_type".to_owned(), "refresh_token".to_owned()),
        (
            "refresh_token".to_owned(),
            "carried-refresh-token".to_owned(),
        ),
    ])
}

fn headers() -> BTreeMap<String, String> {
    BTreeMap::from([(
        "Authorization".to_owned(),
        "Basic Y2xpZW50LWlkOmNsaWVudC1zZWNyZXQ=".to_owned(),
    )])
}

fn sent_calls(transport: &GrantPostTransport<ScriptedPost>) -> Vec<SentPost> {
    transport
        .post
        .calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

#[rstest]
#[case::bad_request(400_u16)]
#[case::unauthorized(401_u16)]
#[case::rate_limited(429_u16)]
#[case::last_client_error(499_u16)]
#[tokio::test]
async fn a_client_error_is_rejected_with_its_status(#[case] status: u16) {
    let transport = answering(GrantAnswer::Answered {
        status,
        body: Vec::new(),
    });
    let failure = transport
        .post(TOKEN_ENDPOINT, &form(), &headers())
        .await
        .expect_err("a 4xx is the IdP refusing this grant");
    assert_eq!(failure.kind, RefreshFailureKind::Rejected);
    assert_eq!(
        failure.detail,
        format!("the IdP refused the refresh with status {status}")
    );
}

#[rstest]
#[case::server_error(500_u16)]
#[case::service_unavailable(503_u16)]
#[case::redirect(302_u16)]
#[case::moved_permanently(301_u16)]
#[tokio::test]
async fn every_other_status_is_unavailable(#[case] status: u16) {
    let transport = answering(GrantAnswer::Answered {
        status,
        body: Vec::new(),
    });
    let failure = transport
        .post(TOKEN_ENDPOINT, &form(), &headers())
        .await
        .expect_err("a non-4xx failure is transient");
    assert_eq!(failure.kind, RefreshFailureKind::Unavailable);
    assert_eq!(
        failure.detail,
        format!("the IdP token endpoint answered with status {status}")
    );
}

#[tokio::test]
async fn a_successful_object_body_is_returned() {
    let transport = answering(GrantAnswer::Answered {
        status: 200,
        body: br#"{"id_token":"next-token","refresh_token":"rotated","expires_in":3600}"#.to_vec(),
    });
    let body = transport
        .post(TOKEN_ENDPOINT, &form(), &headers())
        .await
        .expect("a 200 object answers");
    assert_eq!(
        body.get("id_token").and_then(JsonValue::as_str),
        Some("next-token")
    );
    assert_eq!(
        body.get("refresh_token").and_then(JsonValue::as_str),
        Some("rotated")
    );
    assert_eq!(body.len(), 3);
}

#[rstest]
#[case::html(b"<html>not json</html>".to_vec())]
#[case::array(b"[1,2,3]".to_vec())]
#[case::json_null(b"null".to_vec())]
#[case::json_string(b"\"a string\"".to_vec())]
#[case::empty(b"".to_vec())]
#[tokio::test]
async fn a_success_body_that_is_not_an_object_is_unavailable(#[case] body: Vec<u8>) {
    let transport = answering(GrantAnswer::Answered { status: 200, body });
    let failure = transport
        .post(TOKEN_ENDPOINT, &form(), &headers())
        .await
        .expect_err("only an object answers");
    assert_eq!(failure.kind, RefreshFailureKind::Unavailable);
    assert_eq!(failure.detail, NON_OBJECT_DETAIL);
}

#[tokio::test]
async fn an_unreachable_endpoint_is_unavailable() {
    let transport = answering(GrantAnswer::Unreachable);
    let failure = transport
        .post(TOKEN_ENDPOINT, &form(), &headers())
        .await
        .expect_err("no answer is transient");
    assert_eq!(failure.kind, RefreshFailureKind::Unavailable);
    assert_eq!(failure.detail, UNREACHABLE_DETAIL);
}

#[tokio::test]
async fn forwards_the_url_form_and_headers_unchanged() {
    let transport = answering(GrantAnswer::Answered {
        status: 200,
        body: b"{}".to_vec(),
    });
    let sent_form = form();
    let sent_headers = headers();
    transport
        .post(TOKEN_ENDPOINT, &sent_form, &sent_headers)
        .await
        .expect("an empty object answers");
    let calls = sent_calls(&transport);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].url, TOKEN_ENDPOINT);
    assert_eq!(calls[0].form, sent_form);
    assert_eq!(calls[0].headers, sent_headers);
}

#[tokio::test]
async fn a_refusal_detail_carries_no_url_form_or_token() {
    let transport = answering(GrantAnswer::Answered {
        status: 400,
        body: b"{\"error\":\"invalid_grant\"}".to_vec(),
    });
    let failure = transport
        .post(TOKEN_ENDPOINT, &form(), &headers())
        .await
        .expect_err("a 400 is a refusal");
    assert_eq!(
        failure.detail,
        "the IdP refused the refresh with status 400"
    );
    assert!(!failure.detail.contains(TOKEN_ENDPOINT));
    assert!(!failure.detail.contains("carried-refresh-token"));
    assert!(!failure.detail.contains("invalid_grant"));
}

#[tokio::test]
async fn an_outage_detail_carries_no_body() {
    let transport = answering(GrantAnswer::Answered {
        status: 500,
        body: b"{\"error\":\"server_error\"}".to_vec(),
    });
    let failure = transport
        .post(TOKEN_ENDPOINT, &form(), &headers())
        .await
        .expect_err("a 500 is an outage");
    assert_eq!(
        failure.detail,
        "the IdP token endpoint answered with status 500"
    );
    assert!(!failure.detail.contains("server_error"));
    assert!(!failure.detail.contains("carried-refresh-token"));
}

#[test]
fn a_body_answer_debug_hides_the_tokens() {
    let answer = GrantAnswer::Answered {
        status: 200,
        body: b"{\"id_token\":\"secret-token\"}".to_vec(),
    };
    let rendered = format!("{answer:?}");
    assert!(!rendered.contains("secret-token"));
    assert!(rendered.contains("status: 200"));
}

#[test]
fn replays_the_sso_refresh_response_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/sso_refresh_response");
    fs::read_dir(corpus)
        .expect("the corpus directory is readable")
        .map(|entry| fs::read(entry.expect("the corpus entry").path()).expect("the corpus file"))
        .for_each(|bytes| match map_answer(200, &bytes) {
            Ok(_body) => {}
            Err(failure) => {
                assert_eq!(failure.kind, RefreshFailureKind::Unavailable);
                assert_eq!(failure.detail, NON_OBJECT_DETAIL);
            }
        });
}
