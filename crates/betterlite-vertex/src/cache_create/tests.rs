#![cfg(test)]

#[path = "tests/support.rs"]
mod support;

use std::collections::BTreeMap;
use std::error::Error as _;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_models::decisions::Json;
use betterlite_prompts::message_content::Message;
use http::Method;
use http::header::CONTENT_TYPE;
use tokio::time::timeout;

use crate::cache_create::error::CacheCreateError;
use crate::cache_create::orchestration::CacheCreateOptions;
use crate::cache_create::records::{CREATED_CACHE_LIMIT, decode_created_cache};
use crate::cache_create::tests::support::{
    HangingTransport, Harness, Reply, ScriptedTransport, created_cache, empty_list_page,
    expected_cache_key, list_page, page, stalling_server, status,
};

const SHORT_MARKED: &str =
    r#"[{"role":"user","content":"hi","cache_control":{"type":"ephemeral"}}]"#;

const MARKED_WITH_TAIL: &str = r#"[{"role":"user","content":"hi","cache_control":{"type":"ephemeral"}},{"role":"user","content":"after"}]"#;

const TAIL_ONLY: &str = r#"[{"role":"user","content":"after"}]"#;

const NO_MARKERS: &str =
    r#"[{"role":"user","content":"hi"},{"role":"assistant","content":"there"}]"#;

const MODEL_TURN: &str = r#"[{"role":"user","content":"hi","cache_control":{"type":"ephemeral"}},{"role":"assistant","content":"there","cache_control":{"type":"ephemeral"}}]"#;

const IMAGE_MARKED: &str = r#"[{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}},{"type":"image_url","image_url":{"url":"https://example.com/a.png"}}]}]"#;

const MAPPED_TOOLS: &str = r#"[{"functionDeclarations":[{"name":"lookup","description":"Find a record","parameters":{"type":"object","properties":{"id":{"type":"string"}}}}]}]"#;

const TOOL_CONFIG: &str = r#"{"functionCallingConfig":{"mode":"AUTO"}}"#;

fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("fixture json")
}

fn mapped_tools() -> Vec<Json> {
    serde_json::from_str(MAPPED_TOOLS).expect("fixture tools")
}

fn with_tools() -> CacheCreateOptions {
    CacheCreateOptions {
        tools: Some(mapped_tools()),
        tool_choice: Some(json(TOOL_CONFIG)),
    }
}

fn long_marked_message() -> String {
    // about 800 cl100k tokens: above the 512 minimum of the vertex_ai-prefixed catalog entry, below the 1024 fallback
    let words = "word ".repeat(800);
    format!(
        r#"[{{"role":"user","content":"{}","cache_control":{{"type":"ephemeral"}}}}]"#,
        words.trim_end()
    )
}

fn eligible(model: &str, messages: &str) -> Harness {
    let mut harness = Harness::new(model, messages);
    // the fixture prompts are far below every model minimum, so the override is what makes them eligible
    harness.minimum_token_override = Some(1);
    harness
}

#[tokio::test]
async fn explicit_name_bypass() {
    let mut harness = Harness::new("gemini-2.5-pro", SHORT_MARKED);
    let mut options = with_tools();
    let transport = ScriptedTransport::new(vec![empty_list_page(), created_cache("never")]);
    harness.cached_content =
        Some("projects/proj/locations/us-central1/cachedContents/1".to_owned());
    let outcome = harness
        .run(&transport, &mut options)
        .await
        .expect("the bypass");
    assert_eq!(outcome.messages, harness.messages);
    assert_eq!(
        outcome.cached_content.as_deref(),
        Some("projects/proj/locations/us-central1/cachedContents/1")
    );
    assert_eq!(options, with_tools());
    assert!(transport.requests().is_empty());
    harness.cached_content = Some(String::new());
    let empty = harness
        .run(&transport, &mut options)
        .await
        .expect("the bypass");
    assert_eq!(empty.cached_content.as_deref(), Some(""));
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn too_short_unchanged() {
    let harness = Harness::new("gemini-2.5-flash", SHORT_MARKED);
    let mut options = with_tools();
    let transport = ScriptedTransport::new(vec![empty_list_page(), created_cache("never")]);
    let outcome = harness
        .run(&transport, &mut options)
        .await
        .expect("the skip");
    assert_eq!(outcome.messages, harness.messages);
    assert_eq!(outcome.cached_content, None);
    assert_eq!(options, with_tools());
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn unmarked_messages_unchanged() {
    let harness = eligible("gemini-2.5-pro", NO_MARKERS);
    let mut options = with_tools();
    let transport = ScriptedTransport::new(vec![empty_list_page(), created_cache("never")]);
    let outcome = harness
        .run(&transport, &mut options)
        .await
        .expect("the skip");
    assert_eq!(outcome.messages, harness.messages);
    assert_eq!(outcome.cached_content, None);
    assert_eq!(options, with_tools());
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn terminal_turn_unchanged() {
    let harness = eligible("gemini-2.5-pro", MODEL_TURN);
    let mut options = with_tools();
    let transport = ScriptedTransport::new(vec![empty_list_page(), created_cache("never")]);
    let outcome = harness
        .run(&transport, &mut options)
        .await
        .expect("the skip");
    assert_eq!(outcome.messages, harness.messages);
    assert_eq!(outcome.cached_content, None);
    assert_eq!(options, with_tools());
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn model_threshold_override() {
    let mut harness = Harness::new("gemini-2.5-pro", SHORT_MARKED);
    let mut options = CacheCreateOptions::default();
    let skipped = ScriptedTransport::new(vec![empty_list_page(), created_cache("never")]);
    let skipped_outcome = harness.run(&skipped, &mut options).await.expect("the skip");
    assert_eq!(skipped_outcome.cached_content, None);
    assert!(skipped.requests().is_empty());
    harness.minimum_token_override = Some(1);
    let transport =
        ScriptedTransport::new(vec![empty_list_page(), created_cache("cachedContents/1")]);
    let outcome = harness
        .run(&transport, &mut options)
        .await
        .expect("the create");
    assert_eq!(outcome.cached_content.as_deref(), Some("cachedContents/1"));
    assert_eq!(transport.methods(), vec![Method::GET, Method::POST]);
}

#[tokio::test]
async fn lookup_hit_strips_tools() {
    let harness = eligible("gemini-2.5-pro", MARKED_WITH_TAIL);
    let mut options = with_tools();
    let cached = &harness.messages[..1];
    let key = expected_cache_key(
        cached,
        options.tools.as_deref(),
        options.tool_choice.as_ref(),
        &harness.model,
    );
    let transport = ScriptedTransport::new(vec![list_page(&key, "cachedContents/abc")]);
    let outcome = harness
        .run(&transport, &mut options)
        .await
        .expect("the hit");
    assert_eq!(
        outcome.cached_content.as_deref(),
        Some("cachedContents/abc")
    );
    assert_eq!(
        outcome.messages,
        serde_json::from_str::<Vec<Message>>(TAIL_ONLY).expect("fixture messages")
    );
    assert_eq!(options, CacheCreateOptions::default());
    assert_eq!(transport.methods(), vec![Method::GET]);
}

#[tokio::test]
async fn miss_posts_null_tools() {
    let mut harness = eligible("gemini-2.5-pro", SHORT_MARKED);
    harness.extra_headers = BTreeMap::from([(
        "content-type".to_owned(),
        "application/vnd.betterlite+json".to_owned(),
    )]);
    let mut options = CacheCreateOptions::default();
    let key = expected_cache_key(&harness.messages, None, None, &harness.model);
    let transport =
        ScriptedTransport::new(vec![empty_list_page(), created_cache("cachedContents/7")]);
    let outcome = harness
        .run(&transport, &mut options)
        .await
        .expect("the create");
    assert_eq!(outcome.cached_content.as_deref(), Some("cachedContents/7"));
    assert!(outcome.messages.is_empty());
    assert_eq!(options, CacheCreateOptions::default());
    let requests = transport.requests();
    assert_eq!(transport.methods(), vec![Method::GET, Method::POST]);
    assert_eq!(requests[1].url, requests[0].url);
    let Json::Object(fields) =
        serde_json::from_slice::<Json>(&requests[1].body).expect("the create body")
    else {
        panic!("the create body is an object");
    };
    assert_eq!(fields.get("displayName"), Some(&Json::Text(key)));
    assert_eq!(fields.get("tools"), Some(&Json::Null));
    assert!(fields.get("toolConfig").is_none());
    assert_eq!(
        requests[1]
            .headers
            .get(CONTENT_TYPE)
            .expect("a content type"),
        "application/vnd.betterlite+json"
    );
}

#[tokio::test]
async fn miss_posts_mapped_tools() {
    let harness = eligible("gemini-2.5-pro", SHORT_MARKED);
    let mut options = with_tools();
    let key = expected_cache_key(
        &harness.messages,
        options.tools.as_deref(),
        options.tool_choice.as_ref(),
        &harness.model,
    );
    let transport =
        ScriptedTransport::new(vec![empty_list_page(), created_cache("cachedContents/8")]);
    let outcome = harness
        .run(&transport, &mut options)
        .await
        .expect("the create");
    assert_eq!(outcome.cached_content.as_deref(), Some("cachedContents/8"));
    assert_eq!(options, CacheCreateOptions::default());
    let requests = transport.requests();
    let Json::Object(fields) =
        serde_json::from_slice::<Json>(&requests[1].body).expect("the create body")
    else {
        panic!("the create body is an object");
    };
    assert_eq!(fields.get("displayName"), Some(&Json::Text(key)));
    assert_eq!(
        fields.get("model"),
        Some(&Json::Text(
            "projects/proj/locations/us-central1/publishers/google/models/gemini-2.5-pro"
                .to_owned()
        ))
    );
    assert_eq!(fields.get("tools"), Some(&Json::List(mapped_tools())));
    assert_eq!(fields.get("toolConfig"), Some(&json(TOOL_CONFIG)));
}

#[tokio::test]
async fn provider_prefix_selects_the_model_minimum() {
    let harness = Harness::new("claude-sonnet-5-5@default", &long_marked_message());
    let mut options = CacheCreateOptions::default();
    let transport =
        ScriptedTransport::new(vec![empty_list_page(), created_cache("cachedContents/9")]);
    let outcome = harness
        .run(&transport, &mut options)
        .await
        .expect("the create");
    assert_eq!(outcome.cached_content.as_deref(), Some("cachedContents/9"));
    assert_eq!(transport.methods(), vec![Method::GET, Method::POST]);
}

#[tokio::test]
async fn missing_name_is_not_repaired() {
    let harness = eligible("gemini-2.5-pro", SHORT_MARKED);
    let mut options = CacheCreateOptions::default();
    let transport = ScriptedTransport::new(vec![
        empty_list_page(),
        page(r#"{"displayName":"cache-key","model":"models/gemini-2.5-pro"}"#),
    ]);
    let outcome = harness
        .run(&transport, &mut options)
        .await
        .expect("the create");
    assert_eq!(outcome.cached_content, None);
}

#[tokio::test]
async fn create_status_is_preserved() {
    let harness = eligible("gemini-2.5-pro", SHORT_MARKED);
    let mut options = CacheCreateOptions::default();
    let transport =
        ScriptedTransport::new(vec![empty_list_page(), status(400, "bad cached content")]);
    let error = harness
        .run(&transport, &mut options)
        .await
        .expect_err("the create");
    assert!(matches!(error, CacheCreateError::Http { status: 400, .. }));
    assert_eq!(error.status_code(), 400);
    assert!(error.to_string().contains("bad cached content"));
}

#[tokio::test]
async fn list_status_is_preserved() {
    let harness = eligible("gemini-2.5-pro", SHORT_MARKED);
    let mut options = CacheCreateOptions::default();
    let transport = ScriptedTransport::new(vec![status(429, "slow down")]);
    let error = harness
        .run(&transport, &mut options)
        .await
        .expect_err("the list");
    assert!(matches!(error, CacheCreateError::Endpoint { .. }));
    assert_eq!(error.status_code(), 429);
}

#[tokio::test]
async fn transport_error_source_is_preserved() {
    let harness = eligible("gemini-2.5-pro", SHORT_MARKED);
    let mut options = CacheCreateOptions::default();
    let transport = ScriptedTransport::new(vec![
        empty_list_page(),
        Reply::Failure(TransportError::Closed),
    ]);
    let error = harness
        .run(&transport, &mut options)
        .await
        .expect_err("the create");
    assert!(matches!(error, CacheCreateError::Transport { .. }));
    assert!(error.source().is_some());
    assert_eq!(error.status_code(), 500);
}

#[tokio::test]
async fn transport_cancellation() {
    let harness = eligible("gemini-2.5-pro", SHORT_MARKED);
    // the first count loads the tokenizer, so the timed run below reaches the transport before its deadline
    let mut warm_options = CacheCreateOptions::default();
    let warm = ScriptedTransport::new(vec![empty_list_page(), created_cache("cachedContents/0")]);
    harness
        .run(&warm, &mut warm_options)
        .await
        .expect("the warm-up");
    let mut options = CacheCreateOptions::default();
    let transport = HangingTransport::default();
    let hung = timeout(
        Duration::from_millis(200),
        harness.run(&transport, &mut options),
    );
    assert!(hung.await.is_err(), "the hung lookup ends with its caller");
    assert!(transport.was_dropped());
    assert_eq!(transport.request_count(), 1);
}

#[tokio::test]
async fn timeout_408() {
    let server = stalling_server().await;
    let mut harness = eligible("gemini-2.5-pro", SHORT_MARKED);
    harness.api_base = Some(server.base.clone());
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("a proxy policy"),
    );
    let limits = TransportLimits {
        read_timeout: Duration::from_millis(500),
        ..TransportLimits::DEFAULT
    };
    let client = ProxiedClient::build(base_builder(), &policy, limits).expect("a fixture client");
    let mut options = CacheCreateOptions::default();
    let error = harness
        .run(&client, &mut options)
        .await
        .expect_err("the create");
    assert!(matches!(error, CacheCreateError::Timeout { .. }));
    assert_eq!(error.status_code(), 408);
    assert_eq!(server.seen.lock().expect("fixture requests").len(), 2);
}

#[tokio::test]
async fn image_content_skips_caching() {
    let harness = eligible("gemini-2.5-pro", IMAGE_MARKED);
    let mut options = CacheCreateOptions::default();
    let transport = ScriptedTransport::new(vec![empty_list_page(), created_cache("never")]);
    let outcome = harness
        .run(&transport, &mut options)
        .await
        .expect("the skip");
    assert_eq!(outcome.cached_content, None);
    assert_eq!(outcome.messages, harness.messages);
    assert!(transport.requests().is_empty());
}

#[test]
fn created_cache_limit_boundary() {
    assert_eq!(CREATED_CACHE_LIMIT, 16 * 1024 * 1024);
    let at_limit = vec![b' '; CREATED_CACHE_LIMIT];
    assert!(matches!(
        decode_created_cache(&at_limit),
        Err(CacheCreateError::Decode { .. })
    ));
    let over_limit = vec![b' '; CREATED_CACHE_LIMIT + 1];
    assert!(matches!(
        decode_created_cache(&over_limit),
        Err(CacheCreateError::TooLarge {
            limit: CREATED_CACHE_LIMIT
        })
    ));
}

#[test]
fn replays_the_cache_created_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/cache_created");
    fs::read_dir(corpus)
        .expect("corpus folder")
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path
                .file_name()
                .and_then(OsStr::to_str)
                .expect("utf-8 file name");
            let decoded = decode_created_cache(&bytes);
            if name.starts_with("valid") {
                decoded.expect("this seed is a created cache");
            } else {
                assert!(decoded.is_err(), "{name}");
            }
            Ok::<(), io::Error>(())
        })
        .expect("readable corpus");
}
