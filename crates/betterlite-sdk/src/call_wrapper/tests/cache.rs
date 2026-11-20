use betterlite_models::decisions::Json;

use super::support::Stub;
use super::support::cache_all_embeddings;
use super::support::cache_hit;
use super::support::cache_partial_embedding;
use super::support::cache_with_preset_key;
use super::support::chat_request;
use super::support::fixture;
use super::support::request;
use super::support::response;
use super::support::responses_stream;
use super::support::run_sync;
use super::support::value;
use crate::call_setup::RequestMap;
use crate::call_wrapper::cache::CacheMode;
use crate::call_wrapper::state::WrapperOptions;

fn embedding_request() -> RequestMap {
    request(r#"{"model":"text-embedding-3-small","input":["cached","uncached"]}"#)
}

#[test]
fn sync_cache_hit_returns_the_stored_response_without_the_stub() {
    let mut fixture = fixture().with_cache();
    let cached = response(value(r#"{"cached":true}"#));
    fixture.cache.set_found(cache_hit(cached.clone()));
    let stub = Stub::new("completion");
    let outcome = run_sync(&mut fixture, &stub, chat_request("{}")).expect("the cache answers");
    assert_eq!(outcome, cached);
    assert_eq!(stub.call_count(), 0);
    assert_eq!(fixture.cache.lookups(), vec![CacheMode::Sync]);
    assert!(fixture.cache.stored().is_empty());
}

#[test]
fn sync_cache_miss_calls_the_stub_and_stores() {
    let mut fixture = fixture().with_cache();
    let body = value(r#"{"id":"chat-1"}"#);
    let stub = Stub::responding("completion", response(body.clone()));
    run_sync(&mut fixture, &stub, chat_request("{}")).expect("the stub answers");
    assert_eq!(stub.call_count(), 1);
    assert_eq!(fixture.cache.stored(), vec![response(body)]);
}

#[test]
fn sync_caching_false_skips_the_lookup() {
    let mut fixture = fixture().with_cache();
    let stub = Stub::new("completion");
    run_sync(&mut fixture, &stub, chat_request(r#"{"caching":false}"#)).expect("the stub answers");
    assert!(fixture.cache.lookups().is_empty());
    assert_eq!(stub.call_count(), 1);
}

#[test]
fn sync_no_cache_flag_skips_the_lookup() {
    let mut fixture = fixture().with_cache();
    let stub = Stub::new("completion");
    run_sync(
        &mut fixture,
        &stub,
        chat_request(r#"{"cache":{"no-cache":true}}"#),
    )
    .expect("the stub answers");
    assert!(fixture.cache.lookups().is_empty());
    assert_eq!(stub.call_count(), 1);
}

#[tokio::test]
async fn async_cache_hit_with_a_converted_stream_marks_the_stream() {
    let mut fixture = fixture().with_cache();
    let cached = responses_stream(value(r#"{"cached":true}"#));
    fixture.cache.set_found(cache_hit(cached.clone()));
    let stub = Stub::new("acompletion");
    let (handle, _prepared) = fixture
        .wrapper
        .start_logging(
            "acompletion",
            chat_request(r#"{"litellm_call_id":"call-1"}"#),
            true,
        )
        .expect("the logging state builds");
    let outcome = fixture
        .wrapper
        .call(
            &stub,
            chat_request("{}"),
            WrapperOptions {
                logging: Some(handle.clone()),
                internal_call: false,
            },
        )
        .await
        .expect("the cache answers");
    assert_eq!(outcome, cached);
    assert_eq!(stub.call_count(), 0);
    let state = handle.state.lock().expect("the state lock is free");
    assert_eq!(state.stream, Some(true));
    assert_eq!(
        state.model_call_details.get("stream"),
        Some(&Json::Bool(true))
    );
}

#[tokio::test]
async fn async_all_embeddings_cached_returns_the_final_embedding() {
    let mut fixture = fixture().with_cache();
    let embedding = value(r#"{"data":[[1,2]]}"#);
    fixture
        .cache
        .set_found(cache_all_embeddings(embedding.clone()));
    let stub = Stub::new("aembedding");
    let outcome = fixture
        .wrapper
        .call(&stub, embedding_request(), WrapperOptions::default())
        .await
        .expect("the cache answers");
    assert_eq!(outcome, response(embedding));
    assert_eq!(stub.call_count(), 0);
}

#[tokio::test]
async fn async_partial_embedding_hit_stores_and_combines() {
    let mut fixture = fixture().with_cache();
    fixture.cache.set_found(cache_partial_embedding(
        value(r#"{"data":[[1,2]]}"#),
        value(r#"["uncached"]"#),
    ));
    let stub = Stub::responding("aembedding", response(value(r#"{"data":[[3,4]]}"#)));
    let outcome = fixture
        .wrapper
        .call(&stub, embedding_request(), WrapperOptions::default())
        .await
        .expect("the stub answers");
    assert_eq!(outcome, response(value(r#"{"data":[[3,4]]}"#)));
    let calls = stub.calls();
    let seen = calls.first().expect("the stub saw one request");
    assert_eq!(seen.get("input"), Some(&value(r#"["uncached"]"#)));
    assert_eq!(fixture.cache.stored().len(), 1);
    assert!(fixture.cache.was_combined());
    assert_eq!(fixture.metadata.count(), 0);
    assert_eq!(
        fixture.logging.events(),
        vec![String::from("sync_callbacks:embeddings")]
    );
    assert_eq!(fixture.scheduler.count(), 1);
}

#[tokio::test]
async fn async_preset_cache_key_lands_on_the_logging_state() {
    let mut fixture = fixture().with_cache();
    fixture.cache.set_found(cache_with_preset_key("preset-1"));
    let stub = Stub::new("acompletion");
    let (handle, _prepared) = fixture
        .wrapper
        .start_logging(
            "acompletion",
            chat_request(r#"{"litellm_call_id":"call-1"}"#),
            true,
        )
        .expect("the logging state builds");
    fixture
        .wrapper
        .call(
            &stub,
            chat_request("{}"),
            WrapperOptions {
                logging: Some(handle.clone()),
                internal_call: false,
            },
        )
        .await
        .expect("the stub answers");
    let state = handle.state.lock().expect("the state lock is free");
    assert_eq!(
        state.litellm_params.get("preset_cache_key"),
        Some(&Json::Text(String::from("preset-1")))
    );
}

#[tokio::test]
async fn async_lookup_uses_the_async_mode() {
    let mut fixture = fixture().with_cache();
    let stub = Stub::new("acompletion");
    fixture
        .wrapper
        .call(&stub, chat_request("{}"), WrapperOptions::default())
        .await
        .expect("the stub answers");
    assert_eq!(fixture.cache.lookups(), vec![CacheMode::Async]);
    assert_eq!(stub.call_count(), 1);
}
