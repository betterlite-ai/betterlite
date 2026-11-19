use std::time::Duration;

use betterlite_cache::cache_key::value::{KeyObject, KeyValue};
use betterlite_cache::handler::HandlerError;
use betterlite_cache::handler::state::CachingHandler;
use betterlite_cache::types::call_type::CachingSupportedCallType;

use super::support::{
    Fixture, Tokens, cached_chat, chat_kwargs, computed_key, handler_named, lookup, object,
    perf_clock, services, text, worker,
};
use crate::caching::lookup::async_lookup::async_get_cache;
use crate::caching::lookup::error::LookupError;
use crate::caching::lookup::sync_lookup::sync_get_cache;

// The async eligibility table: the caching switch, whether a cache is configured, and what the caller gets back
fn gated(caching: Option<KeyValue>) -> KeyObject {
    let mut request = chat_kwargs();
    if let Some(value) = caching {
        request.insert("caching".to_owned(), value);
    }
    request
}

fn no_cache_controlled(request: &mut KeyObject) {
    request.insert(
        "cache".to_owned(),
        object(&[("no-cache", KeyValue::Bool(true))]),
    );
}

fn handler_for(name: &'static str, request: &KeyObject) -> CachingHandler {
    handler_named(name, &["model", "messages"], request.clone())
}

#[rstest::rstest]
#[case::a_missing_switch_reads_a_configured_cache(None, true, true, true, true)]
#[case::a_null_switch_reads_a_configured_cache(Some(KeyValue::Null), true, true, true, true)]
#[case::a_true_switch_reads_a_configured_cache(Some(KeyValue::Bool(true)), true, true, true, true)]
#[case::a_true_switch_without_a_cache_returns_an_empty_response(
    Some(KeyValue::Bool(true)),
    false,
    true,
    false,
    false
)]
#[case::a_missing_switch_without_a_cache_returns_none(None, false, false, false, false)]
#[case::a_false_switch_leaves_the_cache_unread(
    Some(KeyValue::Bool(false)),
    true,
    false,
    false,
    false
)]
#[case::a_false_switch_without_a_cache_returns_none(
    Some(KeyValue::Bool(false)),
    false,
    false,
    false,
    false
)]
#[tokio::test]
async fn the_async_gate_decides_whether_the_cache_is_read(
    #[case] caching: Option<KeyValue>,
    #[case] cache_configured: bool,
    #[case] response_present: bool,
    #[case] read_attempted: bool,
    #[case] hit_present: bool,
) {
    let fixture = Fixture::new();
    let request_kwargs = gated(caching);
    fixture.backend.seed(
        &computed_key(&fixture.cache, &request_kwargs),
        cached_chat(),
    );
    let mut handler = handler_for("acompletion", &request_kwargs);
    let source = fixture.source();
    let resolution = fixture.resolution.context();
    let tokens = Tokens::default();
    let worker = worker();
    let services = services(
        cache_configured.then_some(&source),
        &resolution,
        &worker,
        &tokens,
        perf_clock(&[Duration::ZERO, Duration::ZERO]),
    );
    let outcome = async_get_cache(
        &mut handler,
        lookup(CachingSupportedCallType::Acompletion, request_kwargs),
        fixture.hit_logging(),
        &services,
        &fixture.conversions,
    )
    .await
    .expect("the eligibility gate runs");
    assert_eq!(outcome.response.is_some(), response_present);
    assert_eq!(!fixture.backend.reads().is_empty(), read_attempted);
    if let Some(response) = outcome.response {
        assert_eq!(response.cached_result.is_some(), hit_present);
    }
}

#[rstest::rstest]
#[case::a_caching_switch_that_is_not_a_boolean(
    "caching",
    KeyValue::String("yes".to_owned()),
    "caching"
)]
#[case::a_cache_control_that_is_not_an_object("cache", KeyValue::String("yes".to_owned()), "cache")]
#[case::a_no_cache_control_that_is_not_a_boolean(
    "cache",
    object(&[("no-cache", text("yes"))]),
    "cache.no-cache"
)]
#[tokio::test]
async fn a_malformed_cache_control_is_refused(
    #[case] name: &str,
    #[case] value: KeyValue,
    #[case] field: &str,
) {
    let fixture = Fixture::new();
    let mut request_kwargs = chat_kwargs();
    request_kwargs.insert(name.to_owned(), value);
    let mut handler = handler_for("acompletion", &request_kwargs);
    let source = fixture.source();
    let resolution = fixture.resolution.context();
    let tokens = Tokens::default();
    let worker = worker();
    let services = services(
        Some(&source),
        &resolution,
        &worker,
        &tokens,
        perf_clock(&[Duration::ZERO, Duration::ZERO]),
    );
    let error = match async_get_cache(
        &mut handler,
        lookup(CachingSupportedCallType::Acompletion, request_kwargs),
        fixture.hit_logging(),
        &services,
        &fixture.conversions,
    )
    .await
    {
        Ok(_outcome) => panic!("the malformed control is refused"),
        Err(error) => error,
    };
    let LookupError::Request {
        source: HandlerError::FieldType { field: refused, .. },
    } = error
    else {
        panic!("the malformed request reaches the caller as a field error");
    };
    assert_eq!(refused, field);
    assert!(fixture.backend.reads().is_empty());
}

#[tokio::test]
async fn an_async_no_cache_control_returns_no_response() {
    let fixture = Fixture::new();
    let mut request_kwargs = chat_kwargs();
    no_cache_controlled(&mut request_kwargs);
    fixture.backend.seed(
        &computed_key(&fixture.cache, &request_kwargs),
        cached_chat(),
    );
    let mut handler = handler_for("acompletion", &request_kwargs);
    let source = fixture.source();
    let resolution = fixture.resolution.context();
    let tokens = Tokens::default();
    let worker = worker();
    let services = services(
        Some(&source),
        &resolution,
        &worker,
        &tokens,
        perf_clock(&[Duration::ZERO, Duration::ZERO]),
    );
    let outcome = async_get_cache(
        &mut handler,
        lookup(CachingSupportedCallType::Acompletion, request_kwargs),
        fixture.hit_logging(),
        &services,
        &fixture.conversions,
    )
    .await
    .expect("the no-cache control is read");
    assert!(outcome.response.is_none());
    assert!(fixture.backend.reads().is_empty());
    assert!(outcome.logging.is_some());
}

#[tokio::test]
async fn an_unsupported_call_type_never_reads_the_cache() {
    let fixture = Fixture::new();
    let request_kwargs = chat_kwargs();
    fixture.backend.seed(
        &computed_key(&fixture.cache, &request_kwargs),
        cached_chat(),
    );
    let mut handler = handler_for("image_generation", &request_kwargs);
    let source = fixture.source();
    let resolution = fixture.resolution.context();
    let tokens = Tokens::default();
    let worker = worker();
    let services = services(
        Some(&source),
        &resolution,
        &worker,
        &tokens,
        perf_clock(&[Duration::ZERO, Duration::ZERO]),
    );
    let outcome = async_get_cache(
        &mut handler,
        lookup(CachingSupportedCallType::Acompletion, request_kwargs),
        fixture.hit_logging(),
        &services,
        &fixture.conversions,
    )
    .await
    .expect("the call type is checked");
    assert!(fixture.backend.reads().is_empty());
    let response = outcome.response.expect("a response is returned");
    assert!(response.cached_result.is_none());
}

#[rstest::rstest]
#[case::a_supported_call_reads_the_cache("completion", true)]
#[case::an_unsupported_call_reads_nothing("image_generation", false)]
#[tokio::test]
async fn the_sync_gate_reads_only_for_a_supported_call(
    #[case] name: &'static str,
    #[case] cached: bool,
) {
    let fixture = Fixture::new();
    let request_kwargs = chat_kwargs();
    fixture.backend.seed(
        &computed_key(&fixture.cache, &request_kwargs),
        cached_chat(),
    );
    let mut handler = handler_for(name, &request_kwargs);
    let source = fixture.source();
    let resolution = fixture.resolution.context();
    let tokens = Tokens::default();
    let worker = worker();
    let services = services(
        Some(&source),
        &resolution,
        &worker,
        &tokens,
        perf_clock(&[Duration::ZERO, Duration::ZERO]),
    );
    let mut logging = fixture.hit_logging();
    let response = sync_get_cache(
        &mut handler,
        lookup(CachingSupportedCallType::Completion, request_kwargs),
        &mut logging,
        &services,
        &fixture.conversions,
    )
    .await
    .expect("the sync gate runs");
    assert_eq!(response.cached_result.is_some(), cached);
    assert_eq!(fixture.backend.reads().is_empty(), !cached);
}

#[tokio::test]
async fn a_sync_lookup_ignores_the_caching_switch() {
    let fixture = Fixture::new();
    let request_kwargs = gated(Some(KeyValue::Bool(false)));
    fixture.backend.seed(
        &computed_key(&fixture.cache, &request_kwargs),
        cached_chat(),
    );
    let mut handler = handler_for("completion", &request_kwargs);
    let source = fixture.source();
    let resolution = fixture.resolution.context();
    let tokens = Tokens::default();
    let worker = worker();
    let services = services(
        Some(&source),
        &resolution,
        &worker,
        &tokens,
        perf_clock(&[Duration::ZERO, Duration::ZERO]),
    );
    let mut logging = fixture.hit_logging();
    let response = sync_get_cache(
        &mut handler,
        lookup(CachingSupportedCallType::Completion, request_kwargs),
        &mut logging,
        &services,
        &fixture.conversions,
    )
    .await
    .expect("the sync lookup runs");
    assert!(response.cached_result.is_some());
}

#[tokio::test]
async fn a_sync_no_cache_control_does_not_stop_the_lookup() {
    let fixture = Fixture::new();
    let mut request_kwargs = chat_kwargs();
    no_cache_controlled(&mut request_kwargs);
    fixture.backend.seed(
        &computed_key(&fixture.cache, &request_kwargs),
        cached_chat(),
    );
    let mut handler = handler_for("completion", &request_kwargs);
    let source = fixture.source();
    let resolution = fixture.resolution.context();
    let tokens = Tokens::default();
    let worker = worker();
    let services = services(
        Some(&source),
        &resolution,
        &worker,
        &tokens,
        perf_clock(&[Duration::ZERO, Duration::ZERO]),
    );
    let mut logging = fixture.hit_logging();
    let response = sync_get_cache(
        &mut handler,
        lookup(CachingSupportedCallType::Completion, request_kwargs),
        &mut logging,
        &services,
        &fixture.conversions,
    )
    .await
    .expect("the sync lookup runs");
    assert!(response.cached_result.is_some());
}

#[tokio::test]
async fn a_sync_lookup_without_a_cache_returns_an_empty_response() {
    let fixture = Fixture::new();
    let request_kwargs = chat_kwargs();
    let mut handler = handler_for("completion", &request_kwargs);
    let resolution = fixture.resolution.context();
    let tokens = Tokens::default();
    let worker = worker();
    let services = services(
        None,
        &resolution,
        &worker,
        &tokens,
        perf_clock(&[Duration::ZERO, Duration::ZERO]),
    );
    let mut logging = fixture.hit_logging();
    let response = sync_get_cache(
        &mut handler,
        lookup(CachingSupportedCallType::Completion, request_kwargs),
        &mut logging,
        &services,
        &fixture.conversions,
    )
    .await
    .expect("the sync lookup runs");
    assert!(response.cached_result.is_none());
    assert!(fixture.backend.reads().is_empty());
}
