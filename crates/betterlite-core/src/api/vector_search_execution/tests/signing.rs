use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use betterlite_models::decisions::Json;

use super::json;
use super::support::{
    DelegatingConfig, Embeddings, Events, Fixture, Recorded, RecordingConfig, Reply,
    SEARCH_RESPONSE, Source, ThreadedSeam, Wire, body_bytes, body_json, caller_headers, params,
    sent_header,
};
use crate::api::vector_search_execution::config::{Params, QueryEmbeddingExecutor, SearchQuery};
use crate::api::vector_search_execution::service::{SearchCall, SearchDeps};
use crate::api::vector_store_http_write::service::ClientKind;

#[derive(Debug, Clone, Copy)]
pub(super) enum SigningCase {
    Ordinary,
    SignedBody(&'static [u8]),
    SyncQueryEmbedding,
    AsyncQueryEmbedding,
    AsyncSearchDefault,
    AsyncQueryDefault,
    QueryEmbeddingNoExecutor,
    AsyncAwsSigning,
    AsyncPlainSigning,
    SyncAwsSigning,
}

#[derive(Debug, Clone, Copy)]
enum SigningThread {
    OffLoop,
    Inline,
}

#[derive(Debug, Clone, Copy)]
enum DefaultCase {
    Search,
    Query,
}

struct SigningFixture {
    query: SearchQuery,
    optional_params: Params,
    litellm_params: Params,
    extra_body: Params,
    caller: BTreeMap<String, String>,
}

pub(super) async fn signing_case(case: SigningCase) {
    match case {
        SigningCase::Ordinary => ordinary_signing().await,
        SigningCase::SignedBody(body) => signed_body_is_sent(body).await,
        SigningCase::SyncQueryEmbedding => query_embedding_transform(ClientKind::Sync).await,
        SigningCase::AsyncQueryEmbedding => {
            query_embedding_transform(ClientKind::Asynchronous).await;
        }
        SigningCase::AsyncSearchDefault => default_case(DefaultCase::Search).await,
        SigningCase::AsyncQueryDefault => default_case(DefaultCase::Query).await,
        SigningCase::QueryEmbeddingNoExecutor => query_transform_no_executor().await,
        SigningCase::AsyncAwsSigning => {
            signed_on_thread(ClientKind::Asynchronous, true, SigningThread::OffLoop).await;
        }
        SigningCase::AsyncPlainSigning => {
            signed_on_thread(ClientKind::Asynchronous, false, SigningThread::Inline).await;
        }
        SigningCase::SyncAwsSigning => {
            signed_on_thread(ClientKind::Sync, true, SigningThread::Inline).await;
        }
    }
}

fn signing_fixture() -> SigningFixture {
    SigningFixture {
        query: SearchQuery::Text("a".to_owned()),
        optional_params: params(r#"{"region":"override"}"#),
        litellm_params: params(
            r#"{"api_base":"http://mock.test/v1","timeout":99,"region":"base"}"#,
        ),
        extra_body: params(r#"{"query":"override"}"#),
        caller: caller_headers(&[("X-Trace", "caller-trace")]),
    }
}

async fn ordinary_signing() {
    let case = signing_fixture();
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
    let source = Source::new(wire.clone());
    let config = RecordingConfig::new("http://mock.test/v1", &events);
    let recorder = config.recorder();
    let deps = SearchDeps {
        config: Arc::new(config),
        source: &source,
        supplied: None,
    };
    let call = SearchCall {
        vector_store_id: "vs_a",
        query: &case.query,
        optional_params: &case.optional_params,
        litellm_params: &case.litellm_params,
        extra_headers: Some(&case.caller),
        extra_body: Some(&case.extra_body),
        timeout: Some(Duration::from_secs(7)),
        embedding_executor: None,
    };
    let mut fixture = Fixture::new("vector_store_search", &events);
    fixture
        .search(&deps, &ThreadedSeam, ClientKind::Sync, &call)
        .await
        .expect("the search succeeds");
    let guard = recorder.lock().unwrap_or_else(PoisonError::into_inner);
    assert_ordinary_records(&guard, &case);
    drop(guard);
    assert_eq!(events.recorded(), ["sign", "log", "send"]);
    assert_ordinary_wire(&wire, &fixture);
    assert_transform_logging(&fixture);
}

fn assert_transform_logging(fixture: &Fixture) {
    assert_eq!(
        fixture.detail("query"),
        Some(&json(r#""recorded-by-transform""#))
    );
    let logged = fixture.history();
    let Json::Object(details) = &logged[0] else {
        panic!("the logger function receives the call metadata");
    };
    assert_eq!(
        details.get("query"),
        Some(&json(r#""recorded-by-transform""#))
    );
    assert!(details.contains_key("additional_args"));
}

fn assert_ordinary_records(recorded: &Recorded, case: &SigningFixture) {
    assert_validations(recorded, &case.litellm_params);
    assert_transform(recorded, case);
    assert_transform_details(recorded);
    assert_sign(recorded);
}

fn assert_transform_details(recorded: &Recorded) {
    assert_eq!(recorded.response_details.len(), 1);
    assert_eq!(
        recorded.response_details[0].get("query"),
        Some(&Json::Text("recorded-by-transform".to_owned()))
    );
    assert!(recorded.response_details[0].contains_key("additional_args"));
}

fn assert_validations(recorded: &Recorded, litellm_params: &Params) {
    assert_eq!(recorded.validations.len(), 1);
    assert!(recorded.validations[0].0.contains_key("X-Trace"));
    assert_eq!(&recorded.validations[0].1, litellm_params);
    assert_eq!(recorded.urls.len(), 1);
    assert_eq!(
        recorded.urls[0].get("api_base"),
        litellm_params.get("api_base")
    );
}

fn assert_transform(recorded: &Recorded, case: &SigningFixture) {
    assert_eq!(recorded.transforms.len(), 1);
    assert!(recorded.query_transforms.is_empty());
    let transform = &recorded.transforms[0];
    assert_eq!(transform.vector_store_id, "vs_a");
    assert_eq!(transform.query, case.query);
    assert_eq!(transform.optional_params, case.optional_params);
    assert_eq!(transform.api_base, "http://mock.test/v1");
    assert_eq!(transform.litellm_params, case.litellm_params);
    assert_eq!(transform.extra_body, Some(case.extra_body.clone()));
    assert!(!transform.executor);
    assert!(!transform.async_entry);
}

fn assert_sign(recorded: &Recorded) {
    assert_eq!(recorded.signs.len(), 1);
    let sign = &recorded.signs[0];
    assert_eq!(
        sign.optional_params.get("region"),
        Some(&Json::Text("override".to_owned()))
    );
    assert_eq!(sign.optional_params.get("timeout"), Some(&json("99")));
    assert_eq!(
        sign.api_base,
        "http://mock.test/v1/vector_stores/vs_a/search"
    );
    assert_eq!(sign.request_data, params(r#"{"query":["a"]}"#));
    assert_eq!(
        sign.headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-provider")
    );
    assert_eq!(
        sign.headers.get("X-Trace").map(String::as_str),
        Some("caller-trace")
    );
}

fn assert_ordinary_wire(wire: &Wire, fixture: &Fixture) {
    let sent = wire.requests();
    assert_eq!(sent_header(&sent[0], "x-signed"), Some("sig-1"));
    assert_eq!(body_json(&sent[0]), json(r#"{"query":["a"]}"#));
    let Some(Json::Object(logged)) = fixture.logged("headers") else {
        panic!("the pre-call logs the headers");
    };
    assert!(logged.contains_key("X-Signed"));
}

async fn signed_body_is_sent(body: &'static [u8]) {
    let case = signing_fixture();
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
    let source = Source::new(wire.clone());
    let config = RecordingConfig::new("http://mock.test/v1", &events).with_signed_body(Some(body));
    let recorder = config.recorder();
    let deps = SearchDeps {
        config: Arc::new(config),
        source: &source,
        supplied: None,
    };
    let call = SearchCall {
        vector_store_id: "vs_a",
        query: &case.query,
        optional_params: &case.optional_params,
        litellm_params: &case.litellm_params,
        extra_headers: None,
        extra_body: None,
        timeout: None,
        embedding_executor: None,
    };
    let mut fixture = Fixture::new("vector_store_search", &events);
    fixture
        .search(&deps, &ThreadedSeam, ClientKind::Asynchronous, &call)
        .await
        .expect("the search succeeds");
    let guard = recorder.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(guard.transforms.len(), 1);
    assert!(guard.transforms[0].async_entry);
    drop(guard);
    let sent = wire.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(body_bytes(&sent[0]), Some(body));
}

async fn query_embedding_transform(mode: ClientKind) {
    let case = signing_fixture();
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
    let source = Source::new(wire.clone());
    let mut config = RecordingConfig::new("http://mock.test/v1", &events);
    config.query_embedding = true;
    let recorder = config.recorder();
    let deps = SearchDeps {
        config: Arc::new(config),
        source: &source,
        supplied: None,
    };
    let embeddings = Embeddings;
    let call = SearchCall {
        vector_store_id: "vs_a",
        query: &case.query,
        optional_params: &case.optional_params,
        litellm_params: &case.litellm_params,
        extra_headers: None,
        extra_body: None,
        timeout: Some(Duration::from_secs(7)),
        embedding_executor: Some(&embeddings),
    };
    let mut fixture = Fixture::new("vector_store_search", &events);
    fixture
        .search(&deps, &ThreadedSeam, mode, &call)
        .await
        .expect("the search succeeds");
    let guard = recorder.lock().unwrap_or_else(PoisonError::into_inner);
    assert_query_embedding(&guard, &case, mode);
    assert_eq!(wire.requests().len(), 1);
}

fn assert_query_embedding(recorded: &Recorded, case: &SigningFixture, mode: ClientKind) {
    assert!(recorded.transforms.is_empty());
    assert_eq!(recorded.query_transforms.len(), 1);
    let transform = &recorded.query_transforms[0];
    assert_eq!(transform.async_entry, mode == ClientKind::Asynchronous);
    assert_eq!(transform.litellm_params.get("timeout"), Some(&json("7")));
    assert_eq!(
        transform.litellm_params.get("region"),
        Some(&Json::Text("base".to_owned()))
    );
    assert_eq!(transform.optional_params, case.optional_params);
    assert!(transform.executor);
    assert_eq!(
        recorded.signs[0].optional_params.get("timeout"),
        Some(&json("99"))
    );
}

async fn query_transform_no_executor() {
    let case = signing_fixture();
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
    let source = Source::new(wire.clone());
    let mut config = RecordingConfig::new("http://mock.test/v1", &events);
    config.query_embedding = true;
    let recorder = config.recorder();
    let deps = SearchDeps {
        config: Arc::new(config),
        source: &source,
        supplied: None,
    };
    let call = SearchCall {
        vector_store_id: "vs_a",
        query: &case.query,
        optional_params: &case.optional_params,
        litellm_params: &case.litellm_params,
        extra_headers: None,
        extra_body: None,
        timeout: None,
        embedding_executor: None,
    };
    let mut fixture = Fixture::new("vector_store_search", &events);
    fixture
        .search(&deps, &ThreadedSeam, ClientKind::Asynchronous, &call)
        .await
        .expect("the search succeeds");
    let guard = recorder.lock().unwrap_or_else(PoisonError::into_inner);
    assert!(!guard.query_transforms[0].executor);
    assert!(guard.query_transforms[0].async_entry);
    assert_eq!(
        guard.query_transforms[0].litellm_params.get("timeout"),
        Some(&Json::Null)
    );
}

async fn signed_on_thread(mode: ClientKind, aws: bool, thread: SigningThread) {
    let caller_thread = std::thread::current().id();
    let case = signing_fixture();
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
    let source = Source::new(wire.clone());
    let mut config = RecordingConfig::new("http://mock.test/v1", &events);
    config.signs_with_aws = aws;
    let recorder = config.recorder();
    let deps = SearchDeps {
        config: Arc::new(config),
        source: &source,
        supplied: None,
    };
    let call = SearchCall {
        vector_store_id: "vs_a",
        query: &case.query,
        optional_params: &case.optional_params,
        litellm_params: &case.litellm_params,
        extra_headers: None,
        extra_body: None,
        timeout: None,
        embedding_executor: None,
    };
    let mut fixture = Fixture::new("vector_store_search", &events);
    fixture
        .search(&deps, &ThreadedSeam, mode, &call)
        .await
        .expect("the search succeeds");
    let guard = recorder.lock().unwrap_or_else(PoisonError::into_inner);
    assert_signed_thread(&guard, thread, caller_thread, mode);
    assert_eq!(wire.requests().len(), 1);
}

fn assert_signed_thread(
    recorded: &Recorded,
    thread: SigningThread,
    caller: std::thread::ThreadId,
    mode: ClientKind,
) {
    assert_eq!(
        recorded.transforms[0].async_entry,
        mode == ClientKind::Asynchronous
    );
    assert_eq!(recorded.signs.len(), 1);
    let signed_on = recorded.threads[0];
    match thread {
        SigningThread::OffLoop => assert_ne!(signed_on, caller),
        SigningThread::Inline => assert_eq!(signed_on, caller),
    }
}

// Neither async transform is overridden here, so the traits' sync-to-async defaults run
async fn default_case(kind: DefaultCase) {
    let caller = std::thread::current().id();
    let case = signing_fixture();
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
    let source = Source::new(wire.clone());
    let config = match kind {
        DefaultCase::Search => DelegatingConfig::new(),
        DefaultCase::Query => DelegatingConfig::new().with_query_embedding(),
    };
    let recorder = config.recorder();
    let embeddings = Embeddings;
    let executor: Option<&dyn QueryEmbeddingExecutor> = match kind {
        DefaultCase::Search => None,
        DefaultCase::Query => Some(&embeddings),
    };
    let timeout = match kind {
        DefaultCase::Search => None,
        DefaultCase::Query => Some(Duration::from_secs(7)),
    };
    let deps = SearchDeps {
        config: Arc::new(config),
        source: &source,
        supplied: None,
    };
    let call = SearchCall {
        vector_store_id: "vs_a",
        query: &case.query,
        optional_params: &case.optional_params,
        litellm_params: &case.litellm_params,
        extra_headers: None,
        extra_body: None,
        timeout,
        embedding_executor: executor,
    };
    let mut fixture = Fixture::new("vector_store_search", &events);
    fixture
        .search(&deps, &ThreadedSeam, ClientKind::Asynchronous, &call)
        .await
        .expect("the default async transform searches");
    let guard = recorder.lock().unwrap_or_else(PoisonError::into_inner);
    match kind {
        DefaultCase::Search => {
            assert_eq!(guard.transforms.len(), 1);
            assert!(!guard.transforms[0].async_entry);
            assert!(guard.query_transforms.is_empty());
        }
        DefaultCase::Query => {
            assert!(guard.transforms.is_empty());
            assert_eq!(guard.query_transforms.len(), 1);
            assert!(!guard.query_transforms[0].async_entry);
            assert!(guard.query_transforms[0].executor);
            assert_eq!(
                guard.query_transforms[0].litellm_params.get("timeout"),
                Some(&json("7"))
            );
        }
    }
    assert_eq!(guard.threads, [caller]);
    let sent = wire.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent_header(&sent[0], "x-signed"), Some("sig-1"));
}
