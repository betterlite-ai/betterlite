use std::sync::Arc;
use std::time::Duration;

use http::Method;

use super::support::{
    Events, Fixture, Reply, SEARCH_RESPONSE, Source, ThreadedSeam, Wire, body_json, params,
    sent_header,
};
use super::{SEARCH_BODY, json, openai_search_config, to_json};
use crate::api::vector_search_execution::config::SearchQuery;
use crate::api::vector_search_execution::service::{SearchCall, SearchDeps};
use crate::api::vector_store_http_write::service::ClientKind;

const TIMEOUT: Duration = Duration::from_secs(7);
const OVERRIDE: &str = r#"{"query":"override"}"#;
const STANDARD_PARAMS: &str = r#"{"max_num_results":2,"rewrite_query":false}"#;
const FILTER_PARAMS: &str = r#"{"filters":{"type":"eq","key":"k","value":"v"},"max_num_results":2,"ranking_options":{"ranker":"default-2024-11-15"},"rewrite_query":true}"#;
const FILTER_BODY: &str = r#"{"query":["a","b"],"filters":{"type":"eq","key":"k","value":"v"},"max_num_results":2,"ranking_options":{"ranker":"default-2024-11-15"},"rewrite_query":true}"#;

#[derive(Debug, Clone, Copy)]
pub(super) enum BodyCase {
    SyncTimeoutExtraBody,
    SyncTimeoutNoExtraBody,
    SyncEmptyExtraBody,
    SyncWithoutTimeout,
    SyncWithFilters,
    AsyncTimeoutExtraBody,
    AsyncTimeoutNoExtraBody,
    AsyncEmptyExtraBody,
    AsyncWithoutTimeout,
    AsyncWithFilters,
}

impl BodyCase {
    const fn input(self) -> (ClientKind, Option<Duration>, Option<&'static str>) {
        match self {
            Self::SyncTimeoutExtraBody => (ClientKind::Sync, Some(TIMEOUT), Some(OVERRIDE)),
            Self::SyncTimeoutNoExtraBody | Self::SyncWithFilters => {
                (ClientKind::Sync, Some(TIMEOUT), None)
            }
            Self::SyncEmptyExtraBody => (ClientKind::Sync, Some(TIMEOUT), Some("{}")),
            Self::SyncWithoutTimeout => (ClientKind::Sync, None, None),
            Self::AsyncTimeoutExtraBody => {
                (ClientKind::Asynchronous, Some(TIMEOUT), Some(OVERRIDE))
            }
            Self::AsyncTimeoutNoExtraBody => (ClientKind::Asynchronous, Some(TIMEOUT), None),
            Self::AsyncEmptyExtraBody => (ClientKind::Asynchronous, Some(TIMEOUT), Some("{}")),
            Self::AsyncWithoutTimeout | Self::AsyncWithFilters => {
                (ClientKind::Asynchronous, None, None)
            }
        }
    }

    const fn params_and_body(self) -> (&'static str, &'static str) {
        match self {
            Self::SyncWithFilters | Self::AsyncWithFilters => (FILTER_PARAMS, FILTER_BODY),
            Self::SyncTimeoutExtraBody
            | Self::SyncTimeoutNoExtraBody
            | Self::SyncEmptyExtraBody
            | Self::SyncWithoutTimeout
            | Self::AsyncTimeoutExtraBody
            | Self::AsyncTimeoutNoExtraBody
            | Self::AsyncEmptyExtraBody
            | Self::AsyncWithoutTimeout => (STANDARD_PARAMS, SEARCH_BODY),
        }
    }
}

pub(super) async fn search_body_case(case: BodyCase) {
    let (mode, timeout, extra) = case.input();
    let (optional_json, expected) = case.params_and_body();
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
    let source = Source::new(wire.clone());
    let deps = SearchDeps {
        config: Arc::new(openai_search_config()),
        source: &source,
        supplied: None,
    };
    let litellm_params = params(r#"{"api_base":"http://mock.test/v1"}"#);
    let optional_params = params(optional_json);
    let query = SearchQuery::Sequence(vec!["a".to_owned(), "b".to_owned()]);
    let extra_body = extra.map(params);
    let call = SearchCall {
        vector_store_id: "vs/a",
        query: &query,
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        extra_headers: None,
        extra_body: extra_body.as_ref(),
        timeout,
        embedding_executor: None,
    };
    let mut fixture = Fixture::new("vector_store_search", &events);
    let answer = fixture
        .search(&deps, &ThreadedSeam, mode, &call)
        .await
        .expect("the search succeeds");
    assert_sent(&wire, timeout, expected);
    assert_eq!(to_json(&answer), json(SEARCH_RESPONSE));
}

fn assert_sent(wire: &Wire, timeout: Option<Duration>, expected: &str) {
    let sent = wire.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, Method::POST);
    assert_eq!(sent[0].url.path(), "/v1/vector_stores/vs%2Fa/search");
    assert_eq!(sent[0].timeout, timeout);
    assert_eq!(body_json(&sent[0]), json(expected));
    assert_eq!(
        sent_header(&sent[0], "content-type"),
        Some("application/json")
    );
}
