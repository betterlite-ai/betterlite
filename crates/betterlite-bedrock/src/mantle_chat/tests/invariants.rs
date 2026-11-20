use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::media::error::MediaError;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::claude_invoke::error::ClaudeInvokeError;
use crate::mantle_chat::environment::{chat_headers, messages_headers};
use crate::mantle_chat::messages::move_betas_into_header;
use crate::mantle_chat::response::transform_response;
use crate::mantle_chat::url::complete_url;
use crate::mantle_inputs::records::MantleGlobals;
use crate::request_policy::{Params as MetadataParams, RequestValue};

use super::cases::{IMAGE_BODY, LIMITS, SYNC_MESSAGES, at, url};
use super::support::{
    ChatSession, Fixture, MapFetcher, Mode, RefusingFetcher, invoke_params, js, map_of,
    messages_of, obj,
};

fn identity(name: &str, region: &str, base: &str) -> MantleGlobals {
    MantleGlobals {
        mantle_api_key: Some(SecretString::from(format!("{name}-key"))),
        mantle_api_base: Some(base.to_owned()),
        mantle_region: None,
        aws_bearer_token: Some(SecretString::from(format!("{name}-token"))),
        aws_region_name: Some(region.to_owned()),
        aws_region: None,
    }
}

fn params_for(name: &str) -> MetadataParams {
    let mut params = MetadataParams::new();
    params.insert(
        "aws_bedrock_project_id".to_owned(),
        RequestValue::Text(name.to_owned()),
    );
    params
}

#[test]
fn mantle_claude_credential_identity_isolation() {
    let alice = identity("alice", "eu-west-1", "https://mantle.alice.test");
    let bob = identity("bob", "us-east-2", "https://mantle.bob.test");
    urls_keep_their_identity(&alice, &bob);
    headers_keep_their_identity();
}

fn urls_keep_their_identity(alice: &MantleGlobals, bob: &MantleGlobals) {
    assert_eq!(
        url(None, "claude", "{}", alice),
        "https://mantle.alice.test/anthropic/v1/messages"
    );
    assert_eq!(
        url(None, "claude", "{}", bob),
        "https://mantle.bob.test/anthropic/v1/messages"
    );
    assert_eq!(
        url(None, "claude", "{}", alice),
        "https://mantle.alice.test/anthropic/v1/messages"
    );
}

fn headers_keep_their_identity() {
    let caller: IndexMap<String, String> =
        IndexMap::from([("x-caller".to_owned(), "kept".to_owned())]);
    let alice_headers = chat_headers(
        &caller,
        &mut invoke_params("{}"),
        &params_for("alice"),
        None,
    )
    .expect("alice's chat headers merge");
    let bob_headers = chat_headers(&caller, &mut invoke_params("{}"), &params_for("bob"), None)
        .expect("bob's chat headers merge");
    assert_eq!(
        alice_headers.get("anthropic-workspace").map(String::as_str),
        Some("alice")
    );
    assert_eq!(
        bob_headers.get("anthropic-workspace").map(String::as_str),
        Some("bob")
    );
    assert!(!caller.contains_key("anthropic-workspace"));
    let key = SecretString::from("alice-key");
    let (merged, base) = messages_headers(
        &caller,
        &params_for("alice"),
        None,
        Some("https://mantle.alice.test"),
        Some(&key),
    )
    .expect("alice's messages headers merge");
    assert_eq!(base.as_deref(), Some("https://mantle.alice.test"));
    assert!(merged.values().all(|value| !value.contains("alice-key")));
    assert_eq!(
        merged.get("anthropic-workspace").map(String::as_str),
        Some("alice")
    );
}

const SECRET: &str = "fixture-secret-credential";
const TOKEN: &str = "fixture-bearer-token";

#[test]
fn mantle_claude_secret_safe_failure() {
    let globals = MantleGlobals {
        mantle_api_key: Some(SecretString::from(SECRET)),
        mantle_api_base: None,
        mantle_region: None,
        aws_bearer_token: Some(SecretString::from(TOKEN)),
        aws_region_name: Some("us-west-2".to_owned()),
        aws_region: None,
    };
    let debug = format!("{globals:?}");
    assert!(!debug.contains(SECRET));
    assert!(!debug.contains(TOKEN));
    let region_error = complete_url(
        None,
        "claude",
        &invoke_params(r#"{"aws_region_name":"NOT A REGION"}"#),
        &globals,
    )
    .expect_err("the region is invalid");
    assert!(!region_error.to_string().contains(SECRET));
    assert!(!format!("{region_error:?}").contains(SECRET));
    let mut headers = IndexMap::new();
    headers.insert("authorization".to_owned(), format!("Bearer {SECRET}"));
    let body = obj(r#"{"anthropic_beta":"beta"}"#);
    let beta_error = move_betas_into_header(&body, &mut headers).expect_err("the beta shape fails");
    assert!(!beta_error.to_string().contains(SECRET));
    assert!(!format!("{beta_error:?}").contains(SECRET));
    let usage_error = transform_response(br#"{"usage":"many"}"#, 200, &LIMITS)
        .expect_err("the usage shape fails");
    assert!(!usage_error.to_string().contains(SECRET));
}

const MEDIA_FETCH_MESSAGES: &str = r#"[{"role":"user","content":[
    {"type":"image_url","image_url":{"url":"http://example.test/pic.png"}},
    {"type":"file","file":{"file_id":"https://files.example.test/f1"}},
    {"type":"document","source":{"type":"url","url":"http://example.test/doc.pdf"}}
]}]"#;

#[test]
fn mantle_claude_media_fetch() {
    let fixture = Fixture::new();
    let mut session = ChatSession::new(&fixture);
    async_media_shapes(&fixture, &mut session);
    sync_media_untouched(&fixture, &mut session);
    refusing_fetcher_fails(&fixture, &mut session);
}

fn async_media_shapes(fixture: &Fixture, session: &mut ChatSession<'_>) {
    let (fetcher, body) = async_media_body(fixture, session);
    async_media_sources(&body);
    assert_eq!(
        fetcher.calls(),
        vec![
            "http://example.test/pic.png",
            "https://files.example.test/f1",
            "http://example.test/doc.pdf"
        ]
    );
    assert_eq!(fetcher.transport_attempts(), 3);
}

fn async_media_body(fixture: &Fixture, session: &mut ChatSession<'_>) -> (MapFetcher, Object) {
    let fetcher = MapFetcher::new(&[
        (
            "http://example.test/pic.png",
            "data:image/png;base64,iVBORw0KGgo=",
        ),
        (
            "https://files.example.test/f1",
            "data:application/pdf;base64,ZmlsZQ==",
        ),
        (
            "http://example.test/doc.pdf",
            "data:application/pdf;base64,JVBERi0xLjQK",
        ),
    ]);
    let request = fixture.chat_request(
        "fixture-plain",
        messages_of(MEDIA_FETCH_MESSAGES),
        map_of(r#"{"max_tokens":64}"#),
    );
    let body = session
        .transform(&request, &fetcher, Mode::Async)
        .expect("the media request transforms");
    (fetcher, body)
}

fn async_media_sources(body: &Object) {
    assert_eq!(
        at(body, &["messages", "0", "content", "0", "source"]),
        &js(IMAGE_BODY)
    );
    assert_eq!(
        at(body, &["messages", "0", "content", "1", "source"]),
        &js(r#"{"data":"ZmlsZQ==","media_type":"application/pdf","type":"base64"}"#)
    );
    assert_eq!(
        at(body, &["messages", "0", "content", "2", "source"]),
        &js(r#"{"data":"JVBERi0xLjQK","media_type":"application/pdf","type":"base64"}"#)
    );
}

fn sync_media_untouched(fixture: &Fixture, session: &mut ChatSession<'_>) {
    let quiet = MapFetcher::default();
    let request = fixture.chat_request(
        "fixture-plain",
        messages_of(SYNC_MESSAGES),
        map_of(r#"{"max_tokens":64}"#),
    );
    let body = session
        .transform(&request, &quiet, Mode::Sync)
        .expect("the sync request transforms");
    assert!(quiet.calls().is_empty());
    assert_eq!(
        at(&body, &["messages", "0", "content", "0", "source"]),
        &js(r#"{"type":"url","url":"https://files.example.test/f1"}"#)
    );
}

fn refusing_fetcher_fails(fixture: &Fixture, session: &mut ChatSession<'_>) {
    let refusing = RefusingFetcher::default();
    let request = fixture.chat_request(
        "fixture-plain",
        messages_of(MEDIA_FETCH_MESSAGES),
        map_of(r#"{"max_tokens":64}"#),
    );
    match session.transform(&request, &refusing, Mode::Async) {
        Ok(_body) => panic!("the refusing fetcher must fail the request"),
        Err(ClaudeInvokeError::Media(MediaError::Fetch(ImageFetchError::Rejected { url }))) => {
            assert_eq!(url, "http://example.test/pic.png");
        }
        Err(other) => panic!("unexpected error: {other}"),
    }
    assert_eq!(refusing.transport_attempts(), 0);
    assert_eq!(refusing.calls(), vec!["http://example.test/pic.png"]);
}
