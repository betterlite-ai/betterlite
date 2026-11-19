use std::collections::BTreeMap;
use std::sync::Arc;

use rstest::rstest;

use super::support::{
    Events, Fixture, RecordingConfig, RefusingSeam, Reply, SEARCH_RESPONSE, Source, ThreadedSeam,
    Wire, caller_headers, params, sent_header,
};
use super::{TEXT_SEARCH_BODY, json, openai_search_config};
use crate::api::vector_search_execution::config::{Params, SearchConfig, SearchQuery};
use crate::api::vector_search_execution::error::SearchError;
use crate::api::vector_search_execution::service::{SearchCall, SearchDeps};
use crate::api::vector_store_http_write::service::{ClientKind, SuppliedClient};

#[derive(Debug, Clone, Copy)]
pub(super) enum HeaderCase {
    CallerHeaders,
    BetaHeader,
    SuppliedClient,
    SuppliedMismatch,
    NullVerify,
    BadName,
    BadValue,
    BadUrl,
    TransformFailure,
    RawRequest,
    SigningFailure,
}

#[rstest]
#[case::caller_headers(HeaderCase::CallerHeaders)]
#[case::beta_header(HeaderCase::BetaHeader)]
#[case::supplied_client(HeaderCase::SuppliedClient)]
#[case::supplied_mismatch(HeaderCase::SuppliedMismatch)]
#[case::null_verify(HeaderCase::NullVerify)]
#[case::bad_name(HeaderCase::BadName)]
#[case::bad_value(HeaderCase::BadValue)]
#[case::bad_url(HeaderCase::BadUrl)]
#[case::transform_failure(HeaderCase::TransformFailure)]
#[case::raw_request(HeaderCase::RawRequest)]
#[case::signing_failure(HeaderCase::SigningFailure)]
#[tokio::test]
async fn search_validated_headers(#[case] case: HeaderCase) {
    header_case(case).await;
}

async fn header_case(case: HeaderCase) {
    match case {
        HeaderCase::CallerHeaders => caller_headers_overwritten().await,
        HeaderCase::BetaHeader => beta_header_added().await,
        HeaderCase::SuppliedClient => supplied_client_used().await,
        HeaderCase::SuppliedMismatch => supplied_mismatch().await,
        HeaderCase::NullVerify => null_verify_absent().await,
        HeaderCase::BadName => invalid_header_case(InvalidCase::BadName).await,
        HeaderCase::BadValue => invalid_header_case(InvalidCase::BadValue).await,
        HeaderCase::BadUrl => invalid_header_case(InvalidCase::BadUrl).await,
        HeaderCase::TransformFailure => transform_failure().await,
        HeaderCase::RawRequest => raw_request_capture().await,
        HeaderCase::SigningFailure => signing_failure().await,
    }
}

const fn search_call<'request>(
    query: &'request SearchQuery,
    optional_params: &'request Params,
    litellm_params: &'request Params,
    extra_headers: Option<&'request BTreeMap<String, String>>,
) -> SearchCall<'request> {
    SearchCall {
        vector_store_id: "vs_a",
        query,
        optional_params,
        litellm_params,
        extra_headers,
        extra_body: None,
        timeout: None,
        embedding_executor: None,
    }
}

fn keyed_params() -> Params {
    params(r#"{"api_base":"http://mock.test/v1","api_key":"sk-p"}"#)
}

async fn caller_headers_overwritten() {
    let query = SearchQuery::Text("a".to_owned());
    let optional_params = params("{}");
    let litellm_params = keyed_params();
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
    let source = Source::new(wire.clone());
    let deps = SearchDeps {
        config: Arc::new(openai_search_config()),
        source: &source,
        supplied: None,
    };
    let caller = caller_headers(&[
        ("Authorization", "Bearer caller"),
        ("Content-Type", "text/plain"),
        ("OpenAI-Beta", "assistants=v1"),
        ("X-Trace", "trace-1"),
    ]);
    let call = search_call(&query, &optional_params, &litellm_params, Some(&caller));
    let mut fixture = Fixture::new("vector_store_search", &events);
    fixture
        .search(&deps, &ThreadedSeam, ClientKind::Sync, &call)
        .await
        .expect("the search succeeds");
    assert_overwritten_headers(&wire);
    assert_pre_call_logged(&fixture, &events);
}

fn assert_overwritten_headers(wire: &Wire) {
    let sent = wire.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent_header(&sent[0], "authorization"), Some("Bearer sk-p"));
    assert_eq!(
        sent_header(&sent[0], "content-type"),
        Some("application/json")
    );
    assert_eq!(sent_header(&sent[0], "openai-beta"), Some("assistants=v1"));
    assert_eq!(sent_header(&sent[0], "x-trace"), Some("trace-1"));
}

fn assert_pre_call_logged(fixture: &Fixture, events: &Events) {
    assert_eq!(events.recorded(), ["log", "send"]);
    assert_eq!(
        fixture.logged("api_base"),
        Some(&json(r#""http://mock.test/v1/vector_stores""#))
    );
    assert_eq!(
        fixture.logged("complete_input_dict"),
        Some(&json(TEXT_SEARCH_BODY))
    );
    assert_eq!(fixture.input(), Some(&json(r#""""#)));
}

async fn beta_header_added() {
    let query = SearchQuery::Text("a".to_owned());
    let optional_params = params("{}");
    let litellm_params = keyed_params();
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
    let source = Source::new(wire.clone());
    let deps = SearchDeps {
        config: Arc::new(openai_search_config()),
        source: &source,
        supplied: None,
    };
    let only_trace = caller_headers(&[("X-Trace", "trace-2")]);
    let call = search_call(&query, &optional_params, &litellm_params, Some(&only_trace));
    let mut fixture = Fixture::new("vector_store_search", &events);
    fixture
        .search(&deps, &ThreadedSeam, ClientKind::Asynchronous, &call)
        .await
        .expect("the search succeeds");
    let sent = wire.requests();
    assert_eq!(sent_header(&sent[0], "openai-beta"), Some("assistants=v2"));
}

async fn supplied_client_used() {
    let query = SearchQuery::Text("a".to_owned());
    let optional_params = params("{}");
    let litellm_params = keyed_params();
    let events = Events::new();
    let pooled = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
    let supplied_wire = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
    let source = Source::new(pooled.clone());
    let supplied = SuppliedClient {
        kind: ClientKind::Sync,
        transport: &supplied_wire,
    };
    let deps = SearchDeps {
        config: Arc::new(openai_search_config()),
        source: &source,
        supplied: Some(supplied),
    };
    let call = search_call(&query, &optional_params, &litellm_params, None);
    let mut fixture = Fixture::new("vector_store_search", &events);
    fixture
        .search(&deps, &ThreadedSeam, ClientKind::Sync, &call)
        .await
        .expect("the search succeeds");
    assert_eq!(supplied_wire.requests().len(), 1);
    assert!(pooled.requests().is_empty());
    assert!(source.calls().is_empty());
}

async fn supplied_mismatch() {
    let query = SearchQuery::Text("a".to_owned());
    let optional_params = params("{}");
    let events = Events::new();
    let pooled = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
    let supplied_wire = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
    let source = Source::new(pooled.clone());
    let supplied = SuppliedClient {
        kind: ClientKind::Sync,
        transport: &supplied_wire,
    };
    let deps = SearchDeps {
        config: Arc::new(openai_search_config()),
        source: &source,
        supplied: Some(supplied),
    };
    let unverified = params(r#"{"api_base":"http://mock.test/v1","ssl_verify":false}"#);
    let call = search_call(&query, &optional_params, &unverified, None);
    let mut fixture = Fixture::new("vector_store_search", &events);
    fixture
        .search(&deps, &ThreadedSeam, ClientKind::Asynchronous, &call)
        .await
        .expect("the search succeeds");
    assert!(supplied_wire.requests().is_empty());
    assert_eq!(pooled.requests().len(), 1);
    assert_eq!(source.calls(), [Some(false)]);
}

async fn null_verify_absent() {
    let query = SearchQuery::Text("a".to_owned());
    let optional_params = params("{}");
    let events = Events::new();
    let pooled = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
    let source = Source::new(pooled.clone());
    let deps = SearchDeps {
        config: Arc::new(openai_search_config()),
        source: &source,
        supplied: None,
    };
    let null_verify = params(r#"{"api_base":"http://mock.test/v1","ssl_verify":null}"#);
    let call = search_call(&query, &optional_params, &null_verify, None);
    let mut fixture = Fixture::new("vector_store_search", &events);
    fixture
        .search(&deps, &ThreadedSeam, ClientKind::Asynchronous, &call)
        .await
        .expect("the search succeeds");
    assert_eq!(source.calls(), [None]);
}

#[derive(Debug, Clone, Copy)]
enum InvalidCase {
    BadName,
    BadValue,
    BadUrl,
}

async fn invalid_header_case(case: InvalidCase) {
    let query = SearchQuery::Text("a".to_owned());
    let empty = params("{}");
    let litellm_params = keyed_params();
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
    let source = Source::new(wire.clone());
    let deps = SearchDeps {
        config: config_for(case, &events),
        source: &source,
        supplied: None,
    };
    let headers = headers_for(case);
    let call = search_call(&query, &empty, &litellm_params, headers.as_ref());
    let mut fixture = Fixture::new("vector_store_search", &events);
    let error = failure_of(
        fixture
            .search(&deps, &ThreadedSeam, ClientKind::Sync, &call)
            .await,
    );
    assert_invalid_error(&error, case);
    assert_eq!(events.recorded(), ["log"]);
    assert!(wire.requests().is_empty());
}

fn config_for(case: InvalidCase, events: &Events) -> Arc<dyn SearchConfig> {
    match case {
        InvalidCase::BadName | InvalidCase::BadValue => Arc::new(openai_search_config()),
        InvalidCase::BadUrl => {
            Arc::new(RecordingConfig::new("http://mock.test/v1", events).with_url("not a url"))
        }
    }
}

fn headers_for(case: InvalidCase) -> Option<BTreeMap<String, String>> {
    match case {
        InvalidCase::BadName => Some(caller_headers(&[("bad name", "ok")])),
        InvalidCase::BadValue => Some(caller_headers(&[("X-Trace", "bad\nvalue")])),
        InvalidCase::BadUrl => None,
    }
}

fn failure_of<T>(result: Result<T, SearchError>) -> SearchError {
    match result {
        Err(error) => error,
        Ok(_) => panic!("the invalid call fails before the send"),
    }
}

fn assert_invalid_error(error: &SearchError, case: InvalidCase) {
    match case {
        InvalidCase::BadName => {
            let SearchError::HeaderName { name, .. } = error else {
                panic!("a header name HTTP cannot carry fails the header name");
            };
            assert_eq!(name, "bad name");
        }
        InvalidCase::BadValue => {
            let SearchError::HeaderValue { name, .. } = error else {
                panic!("a header value HTTP cannot carry fails the header value");
            };
            assert_eq!(name, "X-Trace");
        }
        InvalidCase::BadUrl => {
            let SearchError::RequestUrl { url, .. } = error else {
                panic!("a url that is not a URL fails the request URL");
            };
            assert_eq!(url, "not a url");
        }
    }
}

async fn transform_failure() {
    let query = SearchQuery::Text("a".to_owned());
    let empty = params("{}");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
    let source = Source::new(wire.clone());
    let mut config = RecordingConfig::new("http://mock.test/v1", &events);
    config.transform_failure = Some("max_num_results");
    let deps = SearchDeps {
        config: Arc::new(config),
        source: &source,
        supplied: None,
    };
    let call = search_call(&query, &empty, &empty, None);
    let mut fixture = Fixture::new("vector_store_search", &events);
    let Err(SearchError::Field { field, .. }) = fixture
        .search(&deps, &ThreadedSeam, ClientKind::Sync, &call)
        .await
    else {
        panic!("a transform failure fails the field");
    };
    assert_eq!(field, "max_num_results");
    assert!(events.recorded().is_empty());
    assert!(wire.requests().is_empty());
}

async fn raw_request_capture() {
    let query = SearchQuery::Text("a".to_owned());
    let optional_params = params("{}");
    let litellm_params = keyed_params();
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
    let source = Source::new(wire.clone());
    let deps = SearchDeps {
        config: Arc::new(openai_search_config()),
        source: &source,
        supplied: None,
    };
    let call = search_call(&query, &optional_params, &litellm_params, None);
    let mut fixture = Fixture::new("vector_store_search", &events);
    fixture.state.request_logging.raw_request_only = true;
    let Err(SearchError::Logging(error)) = fixture
        .search(&deps, &ThreadedSeam, ClientKind::Sync, &call)
        .await
    else {
        panic!("a captured raw request fails the pre-call");
    };
    assert_eq!(
        error.to_string(),
        "the raw request was captured instead of sent"
    );
    assert_eq!(events.recorded(), ["log"]);
    assert!(wire.requests().is_empty());
}

async fn signing_failure() {
    let query = SearchQuery::Text("a".to_owned());
    let empty = params("{}");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
    let source = Source::new(wire.clone());
    let deps = SearchDeps {
        config: Arc::new(RecordingConfig::new("http://mock.test/v1", &events)),
        source: &source,
        supplied: None,
    };
    let call = search_call(&query, &empty, &empty, None);
    let mut fixture = Fixture::new("vector_store_search", &events);
    let Err(SearchError::Signing(failure)) = fixture
        .search(&deps, &RefusingSeam, ClientKind::Asynchronous, &call)
        .await
    else {
        panic!("a signing failure fails the signing");
    };
    assert!(failure.to_string().contains("the signing fixture refuses"));
    assert!(events.recorded().is_empty());
    assert!(wire.requests().is_empty());
}
