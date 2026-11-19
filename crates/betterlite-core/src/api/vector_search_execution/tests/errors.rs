use std::sync::Arc;
use std::time::Duration;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_models::decisions::Json;
use http::{HeaderMap, HeaderValue};

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
    BadRequest,
    WrongType,
    NonUtf8,
    Transport,
    Transform,
}

#[derive(Debug, Clone, Copy)]
enum FailureKind {
    Transport,
    Transform,
}

pub(super) async fn errors_case(case: ErrorCase) {
    match case {
        ErrorCase::SyncTimeout => timeout_case(ClientKind::Sync).await,
        ErrorCase::AsyncTimeout => timeout_case(ClientKind::Asynchronous).await,
        ErrorCase::Status => status_case(429, "slow").await,
        ErrorCase::BadRequest => status_case(400, "slow").await,
        ErrorCase::WrongType => wrong_type_case().await,
        ErrorCase::NonUtf8 => non_utf8_case().await,
        ErrorCase::Transport => provider_failure_case(FailureKind::Transport).await,
        ErrorCase::Transform => provider_failure_case(FailureKind::Transform).await,
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

async fn status_case(status: u16, body: &str) {
    let events = Events::new();
    let wire = Wire::replying(
        Reply::json(status, body)
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
        panic!("a non-2xx reply is a provider error");
    };
    assert_masked_headers(&error, status, body);
    assert_eq!(wire.requests().len(), 1);
}

fn assert_masked_headers(error: &BaseLlmException, status: u16, body: &str) {
    assert_eq!(error.status_code, status);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(error.message, body);
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
        Some(&Json::Text(body.len().to_string()))
    );
}

async fn wrong_type_case() {
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
    let source = Source::new(wire.clone());
    let deps = SearchDeps {
        config: Arc::new(openai_search_config()),
        source: &source,
        supplied: None,
    };
    let litellm_params = params(r#"{"api_base":"http://mock.test/v1"}"#);
    let query = SearchQuery::Text("a".to_owned());
    let optional_params = params(r#"{"max_num_results":"2"}"#);
    let call = error_call(&query, &optional_params, &litellm_params);
    let mut fixture = Fixture::new("vector_store_search", &events);
    let Err(SearchError::Field { field, expected }) = fixture
        .search(&deps, &ThreadedSeam, ClientKind::Sync, &call)
        .await
    else {
        panic!("a string where an integer belongs fails the field");
    };
    assert_eq!(field, "max_num_results");
    assert_eq!(expected, "an integer");
    assert!(events.recorded().is_empty());
    assert!(wire.requests().is_empty());
}

async fn non_utf8_case() {
    let events = Events::new();
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-binary",
        HeaderValue::from_bytes(&[0xff]).expect("the header value is opaque"),
    );
    let wire = Wire::replying(
        Reply {
            status: 500,
            headers,
            body: vec![0xff, 0xfe],
        },
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
        panic!("a non-2xx reply is a provider error");
    };
    assert_eq!(error.status_code, 500);
    assert_eq!(error.message, "2 bytes that are not UTF-8");
    assert_eq!(
        error.headers.get("x-binary"),
        Some(&Json::Text("1 bytes that are not UTF-8".to_owned()))
    );
}

async fn provider_failure_case(kind: FailureKind) {
    let events = Events::new();
    let wire = wire_for(kind, &events);
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
        .search(&deps, &ThreadedSeam, mode_for(kind), &call)
        .await
    else {
        panic!("a failed search is a provider error");
    };
    assert_provider_failure(&error, kind);
    assert_eq!(wire.requests().len(), 1);
    assert_eq!(events.recorded(), ["log", "send"]);
}

fn wire_for(kind: FailureKind, events: &Events) -> Wire {
    match kind {
        FailureKind::Transport => Wire::failing(TransportError::Closed, events),
        FailureKind::Transform => Wire::replying(
            Reply::json(200, "\"slow\"").with_header("x-request-id", "req-1"),
            events,
        ),
    }
}

const fn mode_for(kind: FailureKind) -> ClientKind {
    match kind {
        FailureKind::Transport => ClientKind::Asynchronous,
        FailureKind::Transform => ClientKind::Sync,
    }
}

fn assert_provider_failure(error: &BaseLlmException, kind: FailureKind) {
    match kind {
        FailureKind::Transport => {
            assert_eq!(error.status_code, 500);
            assert!(error.status_code_is_synthesized);
            assert!(!error.message.is_empty());
        }
        FailureKind::Transform => {
            assert_eq!(error.status_code, 200);
            assert!(!error.status_code_is_synthesized);
            assert!(error.message.contains("invalid type"));
            assert_eq!(
                error.headers.get("x-request-id"),
                Some(&Json::Text("req-1".to_owned()))
            );
        }
    }
}
