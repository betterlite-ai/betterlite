use std::sync::Arc;

use super::support::{
    Events, Fixture, RecordingConfig, RefusingSeam, Reply, SEARCH_RESPONSE, Source, ThreadedSeam,
    Wire, caller_headers, params,
};
use super::{TEXT_SEARCH_BODY, json, openai_search_config};
use crate::api::vector_search_execution::config::SearchQuery;
use crate::api::vector_search_execution::error::SearchError;
use crate::api::vector_search_execution::service::{SearchCall, SearchDeps};
use crate::api::vector_store_http_write::service::{ClientKind, SuppliedClient};

#[tokio::test]
async fn search_validated_headers() {
    let query = SearchQuery::Text("a".to_owned());
    let optional_params = params("{}");
    let litellm_params = params(r#"{"api_base":"http://mock.test/v1","api_key":"sk-p"}"#);
    {
        // The provider's credentials and content type overwrite the caller's; the beta header and the rest are kept
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
        let call = SearchCall {
            vector_store_id: "vs_a",
            query: &query,
            optional_params: &optional_params,
            litellm_params: &litellm_params,
            extra_headers: Some(&caller),
            extra_body: None,
            timeout: None,
            embedding_executor: None,
        };
        let mut fixture = Fixture::new("vector_store_search", &events);
        fixture
            .search(&deps, &ThreadedSeam, ClientKind::Sync, &call)
            .await
            .expect("the search succeeds");
        {
            let sent = wire.requests();
            assert_eq!(sent.len(), 1);
            let headers = &sent[0].headers;
            let value = |name: &str| {
                headers
                    .get(name)
                    .map(|value| value.to_str().expect("the header is text").to_owned())
            };
            assert_eq!(value("authorization").as_deref(), Some("Bearer sk-p"));
            assert_eq!(value("content-type").as_deref(), Some("application/json"));
            assert_eq!(value("openai-beta").as_deref(), Some("assistants=v1"));
            assert_eq!(value("x-trace").as_deref(), Some("trace-1"));
        }
        // The pre-call runs before the send, with the base URL and the transformed body
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
    {
        // With no beta header from the caller the OpenAI config adds assistants=v2
        let events = Events::new();
        let wire = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
        let source = Source::new(wire.clone());
        let deps = SearchDeps {
            config: Arc::new(openai_search_config()),
            source: &source,
            supplied: None,
        };
        let only_trace = caller_headers(&[("X-Trace", "trace-2")]);
        let call = SearchCall {
            vector_store_id: "vs_a",
            query: &query,
            optional_params: &optional_params,
            litellm_params: &litellm_params,
            extra_headers: Some(&only_trace),
            extra_body: None,
            timeout: None,
            embedding_executor: None,
        };
        let mut fixture = Fixture::new("vector_store_search", &events);
        fixture
            .search(&deps, &ThreadedSeam, ClientKind::Asynchronous, &call)
            .await
            .expect("the search succeeds");
        let sent = wire.requests();
        assert_eq!(
            sent[0]
                .headers
                .get("openai-beta")
                .map(|value| value.to_str().expect("the header is text")),
            Some("assistants=v2")
        );
    }
    {
        // A supplied client of the caller's mode is used and the pool is never asked for one
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
        let call = SearchCall {
            vector_store_id: "vs_a",
            query: &query,
            optional_params: &optional_params,
            litellm_params: &litellm_params,
            extra_headers: None,
            extra_body: None,
            timeout: None,
            embedding_executor: None,
        };
        let mut fixture = Fixture::new("vector_store_search", &events);
        fixture
            .search(&deps, &ThreadedSeam, ClientKind::Sync, &call)
            .await
            .expect("the search succeeds");
        assert_eq!(supplied_wire.requests().len(), 1);
        assert!(pooled.requests().is_empty());
        assert!(source.calls().is_empty());
    }
    {
        // A supplied client of the other mode is ignored, and the pool gets the caller's ssl_verify
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
        let call = SearchCall {
            vector_store_id: "vs_a",
            query: &query,
            optional_params: &optional_params,
            litellm_params: &unverified,
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
        assert!(supplied_wire.requests().is_empty());
        assert_eq!(pooled.requests().len(), 1);
        assert_eq!(source.calls(), [Some(false)]);
    }
    {
        // An explicit null reads as absent
        let events = Events::new();
        let pooled = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
        let source = Source::new(pooled.clone());
        let deps = SearchDeps {
            config: Arc::new(openai_search_config()),
            source: &source,
            supplied: None,
        };
        let null_verify = params(r#"{"api_base":"http://mock.test/v1","ssl_verify":null}"#);
        let call = SearchCall {
            vector_store_id: "vs_a",
            query: &query,
            optional_params: &optional_params,
            litellm_params: &null_verify,
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
        assert_eq!(source.calls(), [None]);
    }
    {
        // A header name HTTP cannot carry fails after the pre-call and sends nothing
        let events = Events::new();
        let wire = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
        let source = Source::new(wire.clone());
        let deps = SearchDeps {
            config: Arc::new(openai_search_config()),
            source: &source,
            supplied: None,
        };
        let bad_name = caller_headers(&[("bad name", "ok")]);
        let call = SearchCall {
            vector_store_id: "vs_a",
            query: &query,
            optional_params: &optional_params,
            litellm_params: &litellm_params,
            extra_headers: Some(&bad_name),
            extra_body: None,
            timeout: None,
            embedding_executor: None,
        };
        let mut fixture = Fixture::new("vector_store_search", &events);
        let Err(SearchError::HeaderName { name, .. }) = fixture
            .search(&deps, &ThreadedSeam, ClientKind::Sync, &call)
            .await
        else {
            panic!("a header name HTTP cannot carry fails the header name");
        };
        assert_eq!(name, "bad name");
        assert_eq!(events.recorded(), ["log"]);
        assert!(wire.requests().is_empty());
    }
    {
        // A header value HTTP cannot carry fails after the pre-call and sends nothing
        let events = Events::new();
        let wire = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
        let source = Source::new(wire.clone());
        let deps = SearchDeps {
            config: Arc::new(openai_search_config()),
            source: &source,
            supplied: None,
        };
        let bad_value = caller_headers(&[("X-Trace", "bad\nvalue")]);
        let call = SearchCall {
            vector_store_id: "vs_a",
            query: &query,
            optional_params: &optional_params,
            litellm_params: &litellm_params,
            extra_headers: Some(&bad_value),
            extra_body: None,
            timeout: None,
            embedding_executor: None,
        };
        let mut fixture = Fixture::new("vector_store_search", &events);
        let Err(SearchError::HeaderValue { name, .. }) = fixture
            .search(&deps, &ThreadedSeam, ClientKind::Sync, &call)
            .await
        else {
            panic!("a header value HTTP cannot carry fails the header value");
        };
        assert_eq!(name, "X-Trace");
        assert!(wire.requests().is_empty());
    }
    {
        // A URL HTTP cannot parse fails after the pre-call and before any connection
        let events = Events::new();
        let wire = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
        let source = Source::new(wire.clone());
        let config = RecordingConfig::new("http://mock.test/v1", &events).with_url("not a url");
        let deps = SearchDeps {
            config: Arc::new(config),
            source: &source,
            supplied: None,
        };
        let empty = params("{}");
        let call = SearchCall {
            vector_store_id: "vs_a",
            query: &query,
            optional_params: &empty,
            litellm_params: &empty,
            extra_headers: None,
            extra_body: None,
            timeout: None,
            embedding_executor: None,
        };
        let mut fixture = Fixture::new("vector_store_search", &events);
        let Err(SearchError::RequestUrl { url, .. }) = fixture
            .search(&deps, &ThreadedSeam, ClientKind::Sync, &call)
            .await
        else {
            panic!("a url that is not a URL fails the request URL");
        };
        assert_eq!(url, "not a url");
        assert_eq!(events.recorded(), ["log"]);
        assert!(wire.requests().is_empty());
    }
    {
        // A transform failure sends nothing and never reaches the pre-call
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
        let empty = params("{}");
        let call = SearchCall {
            vector_store_id: "vs_a",
            query: &query,
            optional_params: &empty,
            litellm_params: &empty,
            extra_headers: None,
            extra_body: None,
            timeout: None,
            embedding_executor: None,
        };
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
    {
        // A pre-call that captures the raw request instead fails before the send
        let events = Events::new();
        let wire = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
        let source = Source::new(wire.clone());
        let deps = SearchDeps {
            config: Arc::new(openai_search_config()),
            source: &source,
            supplied: None,
        };
        let call = SearchCall {
            vector_store_id: "vs_a",
            query: &query,
            optional_params: &optional_params,
            litellm_params: &litellm_params,
            extra_headers: None,
            extra_body: None,
            timeout: None,
            embedding_executor: None,
        };
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
    {
        // A signing failure sends nothing and never reaches the pre-call
        let events = Events::new();
        let wire = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
        let source = Source::new(wire.clone());
        let deps = SearchDeps {
            config: Arc::new(RecordingConfig::new("http://mock.test/v1", &events)),
            source: &source,
            supplied: None,
        };
        let empty = params("{}");
        let call = SearchCall {
            vector_store_id: "vs_a",
            query: &query,
            optional_params: &empty,
            litellm_params: &empty,
            extra_headers: None,
            extra_body: None,
            timeout: None,
            embedding_executor: None,
        };
        let mut fixture = Fixture::new("vector_store_search", &events);
        let Err(SearchError::Signing(failure)) = fixture
            .search(
                &deps,
                &support::RefusingSeam,
                ClientKind::Asynchronous,
                &call,
            )
            .await
        else {
            panic!("a signing failure fails the signing");
        };
        assert!(failure.to_string().contains("the signing fixture refuses"));
        assert!(events.recorded().is_empty());
        assert!(wire.requests().is_empty());
    }
}
