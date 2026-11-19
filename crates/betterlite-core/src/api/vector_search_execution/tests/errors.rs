use std::sync::Arc;
use std::time::Duration;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_models::decisions::Json;

use super::openai_search_config;
use super::support::{
    Events, Fixture, IMPATIENT, ProxiedSource, Reply, SEARCH_RESPONSE, Source, ThreadedSeam, Wire,
    params,
};
use crate::api::vector_search_execution::config::{Params, SearchQuery};
use crate::api::vector_search_execution::error::{SearchError, TIMEOUT_MESSAGE};
use crate::api::vector_search_execution::service::{SearchCall, SearchDeps};
use crate::api::vector_store_http_write::error::BaseLlmException;
use crate::api::vector_store_http_write::service::ClientKind;

#[derive(Debug, Clone, Copy)]
pub(super) enum ErrorCase {
    SyncTimeout,
    AsyncTimeout,
    Status,
    Transport,
    Transform,
}

pub(super) async fn errors_case(case: ErrorCase) {
    match case {
        ErrorCase::SyncTimeout => timeout_case(ClientKind::Sync).await,
        ErrorCase::AsyncTimeout => timeout_case(ClientKind::Asynchronous).await,
        ErrorCase::Status => status_case().await,
        ErrorCase::Transport => transport_case().await,
        ErrorCase::Transform => transform_case().await,
    }
}

const fn error_call<'request>(
    query: &'request SearchQuery,
    optional_params: &'request Params,
    litellm_params: &'request Params,
) -> SearchCall<'request> {
    SearchCall {
        vector_store_id: "vs_a",
        query,
        optional_params,
        litellm_params,
        extra_headers: None,
        extra_body: None,
        timeout: None,
        embedding_executor: None,
    }
}

async fn timeout_case(mode: ClientKind) {
    let serving = serve(
        reply(
            "200 OK",
            &[("content-type", "application/json")],
            SEARCH_RESPONSE,
        ),
        Duration::from_millis(500),
    )
    .expect("the fixture serves");
    let events = Events::new();
    let source = ProxiedSource::new(IMPATIENT);
    let deps = SearchDeps {
        config: Arc::new(openai_search_config()),
        source: &source,
        supplied: None,
    };
    let litellm_params = params(&format!(r#"{{"api_base":"http://{}"}}"#, serving.address));
    let query = SearchQuery::Text("a".to_owned());
    let optional_params = params("{}");
    let call = error_call(&query, &optional_params, &litellm_params);
    let mut fixture = Fixture::new("vector_store_search", &events);
    let Err(SearchError::Provider(error)) = fixture.search(&deps, &ThreadedSeam, mode, &call).await
    else {
        panic!("a timeout is the handler's own provider error");
    };
    assert_eq!(error.status_code, 408);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(error.message, TIMEOUT_MESSAGE);
    assert!(error.headers.is_empty());
    assert!(std::error::Error::source(&error).is_none());
}

async fn status_case() {
    let events = Events::new();
    let wire = Wire::replying(
        Reply::json(429, "slow")
            .with_header("retry-after", "3")
            .with_header("content-type", "text/plain"),
        &events,
    );
    let source = Source::new(wire.clone());
    let deps = SearchDeps {
        config: Arc::new(openai_search_config()),
        source: &source,
        supplied: None,
    };
    let litellm_params = params(r#"{"api_base":"http://mock.test/v1"}"#);
    let query = SearchQuery::Text("a".to_owned());
    let optional_params = params("{}");
    let call = error_call(&query, &optional_params, &litellm_params);
    let mut fixture = Fixture::new("vector_store_search", &events);
    let Err(SearchError::Provider(error)) = fixture
        .search(&deps, &ThreadedSeam, ClientKind::Sync, &call)
        .await
    else {
        panic!("a 429 is a provider error");
    };
    assert_masked_headers(&error);
    assert_eq!(wire.requests().len(), 1);
}

fn assert_masked_headers(error: &BaseLlmException) {
    assert_eq!(error.status_code, 429);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(error.message, "slow");
    assert_eq!(
        error.headers.get("retry-after"),
        Some(&Json::Text("3".to_owned()))
    );
    assert_eq!(
        error.headers.get("content-type"),
        Some(&Json::Text("text/plain".to_owned()))
    );
    assert_eq!(
        error.headers.get("content-length"),
        Some(&Json::Text("4".to_owned()))
    );
}

async fn transport_case() {
    let events = Events::new();
    let wire = Wire::failing(TransportError::Closed, &events);
    let source = Source::new(wire.clone());
    let deps = SearchDeps {
        config: Arc::new(openai_search_config()),
        source: &source,
        supplied: None,
    };
    let litellm_params = params(r#"{"api_base":"http://mock.test/v1"}"#);
    let query = SearchQuery::Text("a".to_owned());
    let optional_params = params("{}");
    let call = error_call(&query, &optional_params, &litellm_params);
    let mut fixture = Fixture::new("vector_store_search", &events);
    let Err(SearchError::Provider(error)) = fixture
        .search(&deps, &ThreadedSeam, ClientKind::Asynchronous, &call)
        .await
    else {
        panic!("a transport failure is a provider error");
    };
    assert_eq!(error.status_code, 500);
    assert!(error.status_code_is_synthesized);
    assert!(!error.message.is_empty());
    assert_eq!(wire.requests().len(), 1);
    assert_eq!(events.recorded(), ["log", "send"]);
}

async fn transform_case() {
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, "\"slow\""), &events);
    let source = Source::new(wire.clone());
    let deps = SearchDeps {
        config: Arc::new(openai_search_config()),
        source: &source,
        supplied: None,
    };
    let litellm_params = params(r#"{"api_base":"http://mock.test/v1"}"#);
    let query = SearchQuery::Text("a".to_owned());
    let optional_params = params("{}");
    let call = error_call(&query, &optional_params, &litellm_params);
    let mut fixture = Fixture::new("vector_store_search", &events);
    let Err(SearchError::Provider(error)) = fixture
        .search(&deps, &ThreadedSeam, ClientKind::Sync, &call)
        .await
    else {
        panic!("a body the transform rejects is a provider error");
    };
    assert_eq!(error.status_code, 200);
    assert!(!error.status_code_is_synthesized);
    assert!(error.message.contains("invalid type"));
    assert_eq!(wire.requests().len(), 1);
}
