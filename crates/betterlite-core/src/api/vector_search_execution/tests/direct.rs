use std::time::Duration;

use super::support::{
    DelegatingStore, DirectStore, Embeddings, Events, Fixture, SEARCH_RESPONSE, params,
};
use super::{json, to_json};
use crate::api::vector_search_execution::config::{Params, SearchQuery};
use crate::api::vector_search_execution::error::SearchError;
use crate::api::vector_search_execution::service::direct::DirectCall;
use crate::api::vector_store_http_write::service::ClientKind;

#[derive(Debug, Clone, Copy)]
pub(super) enum DirectCase {
    SyncSearch,
    AsyncSearch,
    AsyncDefault,
    SyncFailure,
    AsyncFailure,
}

pub(super) async fn direct_case(case: DirectCase) {
    match case {
        DirectCase::SyncSearch => direct_search(ClientKind::Sync).await,
        DirectCase::AsyncSearch => direct_search(ClientKind::Asynchronous).await,
        DirectCase::AsyncDefault => direct_default().await,
        DirectCase::SyncFailure => direct_failure(ClientKind::Sync).await,
        DirectCase::AsyncFailure => direct_failure(ClientKind::Asynchronous).await,
    }
}

const fn direct_call<'request>(
    query: &'request SearchQuery,
    optional_params: &'request Params,
    litellm_params: &'request Params,
    timeout: Option<Duration>,
    executor: Option<&'request Embeddings>,
) -> DirectCall<'request> {
    DirectCall {
        provider: "valkey",
        vector_store_id: "store-a",
        query,
        optional_params,
        litellm_params,
        timeout,
        embedding_executor: executor,
    }
}

async fn direct_search(mode: ClientKind) {
    let litellm_params = params(r#"{"api_key":"store-key","timeout":99}"#);
    let optional_params = params(r#"{"max_num_results":2}"#);
    let query = SearchQuery::Text("a query".to_owned());
    let embeddings = Embeddings;
    let events = Events::new();
    let store = DirectStore::new(false);
    let call = direct_call(
        &query,
        &optional_params,
        &litellm_params,
        Some(Duration::from_secs(7)),
        Some(&embeddings),
    );
    let mut fixture = Fixture::new("vector_store_search", &events);
    let answer = fixture
        .direct(&store, &call, mode)
        .await
        .expect("the direct search succeeds");
    assert_eq!(to_json(&answer), json(SEARCH_RESPONSE));
    assert_direct_observed(&store, &litellm_params, &query, &optional_params, mode);
    assert_direct_logging(&fixture);
    assert_eq!(events.recorded(), ["log"]);
}

fn assert_direct_observed(
    store: &DirectStore,
    litellm_params: &Params,
    query: &SearchQuery,
    optional_params: &Params,
    mode: ClientKind,
) {
    let observed = store.observed();
    assert_eq!(observed.ids, ["store-a"]);
    assert_eq!(observed.queries, [query.clone()]);
    assert_eq!(observed.optional_params, [optional_params.clone()]);
    assert_eq!(observed.timeouts, [Some(Duration::from_secs(7))]);
    assert_eq!(observed.executors, [true]);
    assert_eq!(observed.litellm_params, [litellm_params.clone()]);
    assert_eq!(observed.async_entries, [mode == ClientKind::Asynchronous]);
}

fn assert_direct_logging(fixture: &Fixture) {
    assert_eq!(
        fixture.logged("api_base"),
        Some(&json(r#""valkey://store-a""#))
    );
    assert_eq!(
        fixture.logged("request_str"),
        Some(&json(r#""direct vector store search: valkey://store-a""#))
    );
    assert_eq!(fixture.logged("query"), Some(&json(r#""a query""#)));
    assert_eq!(
        fixture.logged("vector_store_id"),
        Some(&json(r#""store-a""#))
    );
    assert!(fixture.logged("complete_input_dict").is_none());
    assert!(fixture.logged("headers").is_none());
    assert_eq!(fixture.input(), Some(&json(r#""""#)));
}

// The async default must reach the sync execute, which records no async entry
async fn direct_default() {
    let litellm_params = params(r#"{"api_key":"store-key","timeout":99}"#);
    let optional_params = params(r#"{"max_num_results":2}"#);
    let query = SearchQuery::Text("a query".to_owned());
    let events = Events::new();
    let store = DelegatingStore::default();
    let call = direct_call(&query, &optional_params, &litellm_params, None, None);
    let mut fixture = Fixture::new("vector_store_search", &events);
    let answer = fixture
        .direct(&store, &call, ClientKind::Asynchronous)
        .await
        .expect("the default async execute searches");
    assert_eq!(to_json(&answer), json(SEARCH_RESPONSE));
    let observed = store.observed();
    assert_eq!(observed.ids, ["store-a"]);
    assert_eq!(observed.queries, [query.clone()]);
    assert_eq!(observed.optional_params, [optional_params.clone()]);
    assert_eq!(observed.async_entries, [false]);
}

async fn direct_failure(mode: ClientKind) {
    let litellm_params = params(r#"{"api_key":"store-key","timeout":99}"#);
    let optional_params = params(r#"{"max_num_results":2}"#);
    let query = SearchQuery::Text("a query".to_owned());
    let events = Events::new();
    let store = DirectStore::new(true);
    let call = direct_call(&query, &optional_params, &litellm_params, None, None);
    let mut fixture = Fixture::new("vector_store_search", &events);
    let result = fixture.direct(&store, &call, mode).await;
    let Err(SearchError::Direct(error)) = result else {
        panic!("the store's failure is passed through");
    };
    assert_eq!(error.to_string(), "the store refused");
    let observed = store.observed();
    assert_eq!(observed.async_entries, [mode == ClientKind::Asynchronous]);
    assert_eq!(events.recorded(), ["log"]);
}
