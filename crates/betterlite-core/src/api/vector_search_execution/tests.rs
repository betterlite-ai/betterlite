#![cfg(test)]

mod support;

use std::sync::{Arc, PoisonError};
use std::time::Duration;

use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::LiteLlmGlobals;
use http::Method;
use secrecy::SecretString;

use self::support::{
    DirectStore, Embeddings, Events, Fixture, IMPATIENT, ProxiedSource, RecordingConfig, Reply,
    SEARCH_RESPONSE, Source, ThreadedSeam, Wire, body_bytes, body_json, caller_headers, params,
};
use super::config::SearchQuery;
use super::error::{SearchError, TIMEOUT_MESSAGE};
use super::openai::OpenAiSearchConfig;
use super::service::direct::DirectCall;
use super::service::{SearchCall, SearchDeps};
use crate::api::vector_store_http_write::service::{ClientKind, SuppliedClient};

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the expected JSON parses")
}

fn openai_search_config() -> OpenAiSearchConfig {
    OpenAiSearchConfig::new(
        OpenAiCredentialsSettings::default(),
        LiteLlmGlobals::default(),
        Some(SecretString::from("sk-env")),
        None,
    )
}

// The five keys of a search body, in the shape the OpenAI transform builds them
const SEARCH_BODY: &str = r#"{"query":["a","b"],"filters":null,"max_num_results":2,"ranking_options":null,"rewrite_query":false}"#;
const TEXT_SEARCH_BODY: &str = r#"{"query":"a","filters":null,"max_num_results":null,"ranking_options":null,"rewrite_query":null}"#;

#[tokio::test]
async fn search_http_body() {
    for mode in [ClientKind::Sync, ClientKind::Asynchronous] {
        // A caller timeout, an extra body of any shape and no timeout at all leave the five keys as they are
        for (timeout, extra) in [
            (
                Some(Duration::from_secs(7)),
                Some(r#"{"query":"override"}"#),
            ),
            (Some(Duration::from_secs(7)), None),
            (Some(Duration::from_secs(7)), Some("{}")),
            (None, None),
        ] {
            let events = Events::new();
            let wire = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
            let source = Source::new(wire.clone());
            let deps = SearchDeps {
                config: Arc::new(openai_search_config()),
                source: &source,
                supplied: None,
            };
            let litellm_params = params(r#"{"api_base":"http://mock.test/v1"}"#);
            let optional_params = params(r#"{"max_num_results":2,"rewrite_query":false}"#);
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
            let sent = wire.requests();
            assert_eq!(sent.len(), 1);
            assert_eq!(sent[0].method, Method::POST);
            assert_eq!(sent[0].url.path(), "/v1/vector_stores/vs%2Fa/search");
            assert_eq!(sent[0].timeout, timeout);
            assert_eq!(body_json(&sent[0]), json(SEARCH_BODY));
            assert_eq!(
                sent[0]
                    .headers
                    .get("content-type")
                    .map(|value| value.to_str().expect("the header is text")),
                Some("application/json")
            );
            let returned: Json =
                serde_json::from_slice(&serde_json::to_vec(&answer).expect("the response encodes"))
                    .expect("the response is JSON");
            assert_eq!(returned, json(SEARCH_RESPONSE));
        }
    }
}

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

#[tokio::test]
async fn search_signing_and_embedding() {
    let litellm_params =
        params(r#"{"api_base":"http://mock.test/v1","timeout":99,"region":"base"}"#);
    let optional_params = params(r#"{"region":"override"}"#);
    let extra_body = params(r#"{"query":"override"}"#);
    let query = SearchQuery::Text("a".to_owned());
    let caller = caller_headers(&[("X-Trace", "caller-trace")]);
    {
        // An ordinary transform reads the litellm params as they came, and signing merges the optional params over them
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
            query: &query,
            optional_params: &optional_params,
            litellm_params: &litellm_params,
            extra_headers: Some(&caller),
            extra_body: Some(&extra_body),
            timeout: Some(Duration::from_secs(7)),
            embedding_executor: None,
        };
        let mut fixture = Fixture::new("vector_store_search", &events);
        fixture
            .search(&deps, &ThreadedSeam, ClientKind::Sync, &call)
            .await
            .expect("the search succeeds");
        {
            let recorded = recorder.lock().unwrap_or_else(PoisonError::into_inner);
            assert_eq!(recorded.validations.len(), 1);
            assert!(recorded.validations[0].0.contains_key("X-Trace"));
            assert_eq!(recorded.validations[0].1, litellm_params);
            assert_eq!(recorded.urls.len(), 1);
            assert_eq!(
                recorded.urls[0].get("api_base"),
                litellm_params.get("api_base")
            );
            assert_eq!(recorded.transforms.len(), 1);
            assert!(recorded.query_transforms.is_empty());
            assert_eq!(recorded.transforms[0].vector_store_id, "vs_a");
            assert_eq!(recorded.transforms[0].query, query);
            assert_eq!(recorded.transforms[0].optional_params, optional_params);
            assert_eq!(recorded.transforms[0].api_base, "http://mock.test/v1");
            assert_eq!(recorded.transforms[0].litellm_params, litellm_params);
            assert_eq!(recorded.transforms[0].extra_body, Some(extra_body.clone()));
            assert!(!recorded.transforms[0].executor);
            assert_eq!(recorded.signs.len(), 1);
            assert_eq!(
                recorded.signs[0].optional_params.get("region"),
                Some(&Json::Text("override".to_owned()))
            );
            assert_eq!(
                recorded.signs[0].optional_params.get("timeout"),
                Some(&json("99"))
            );
            assert_eq!(
                recorded.signs[0].api_base,
                "http://mock.test/v1/vector_stores/vs_a/search"
            );
            assert_eq!(recorded.signs[0].request_data, params(r#"{"query":["a"]}"#));
            let headers = &recorded.signs[0].headers;
            assert_eq!(
                headers.get("Authorization").map(String::as_str),
                Some("Bearer sk-provider")
            );
            assert_eq!(
                headers.get("X-Trace").map(String::as_str),
                Some("caller-trace")
            );
        }
        // The signed headers reach the wire, and the pre-call runs after signing and before the send
        assert_eq!(events.recorded(), ["sign", "log", "send"]);
        {
            let sent = wire.requests();
            assert_eq!(
                sent[0]
                    .headers
                    .get("x-signed")
                    .map(|value| value.to_str().expect("the header is text")),
                Some("sig-1")
            );
            // A null signed body is the transformed body, serialized once
            assert_eq!(body_json(&sent[0]), json(r#"{"query":["a"]}"#));
        }
        let Some(Json::Object(logged_headers)) = fixture.logged("headers") else {
            panic!("the pre-call logs the headers");
        };
        assert!(logged_headers.contains_key("X-Signed"));
    }
    // A signed body travels exactly as the signer returned it, an empty one included
    for signed in [Some(&b"signed-body"[..]), Some(&b""[..])] {
        let events = Events::new();
        let wire = Wire::replying(Reply::json(200, SEARCH_RESPONSE), &events);
        let source = Source::new(wire.clone());
        let config = RecordingConfig::new("http://mock.test/v1", &events).with_signed_body(signed);
        let deps = SearchDeps {
            config: Arc::new(config),
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
        fixture
            .search(&deps, &ThreadedSeam, ClientKind::Asynchronous, &call)
            .await
            .expect("the search succeeds");
        let sent = wire.requests();
        assert_eq!(sent.len(), 1);
        assert_eq!(body_bytes(&sent[0]), signed);
    }
    // A query-embedding config's transform reads a copy with the caller's timeout, and receives the executor
    for mode in [ClientKind::Sync, ClientKind::Asynchronous] {
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
            query: &query,
            optional_params: &optional_params,
            litellm_params: &litellm_params,
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
        {
            let recorded = recorder.lock().unwrap_or_else(PoisonError::into_inner);
            assert!(recorded.transforms.is_empty());
            assert_eq!(recorded.query_transforms.len(), 1);
            assert_eq!(
                recorded.query_transforms[0].litellm_params.get("timeout"),
                Some(&json("7"))
            );
            assert_eq!(
                recorded.query_transforms[0].litellm_params.get("region"),
                Some(&Json::Text("base".to_owned()))
            );
            assert_eq!(
                recorded.query_transforms[0].optional_params,
                optional_params
            );
            assert!(recorded.query_transforms[0].executor);
            // The signing map is the litellm params, not the timeout-overridden copy
            assert_eq!(
                recorded.signs[0].optional_params.get("timeout"),
                Some(&json("99"))
            );
        }
        assert_eq!(wire.requests().len(), 1);
    }
    {
        // Without a caller executor the query transform is handed none, and no caller timeout is a null
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
            .search(&deps, &ThreadedSeam, ClientKind::Asynchronous, &call)
            .await
            .expect("the search succeeds");
        let recorded = recorder.lock().unwrap_or_else(PoisonError::into_inner);
        assert!(!recorded.query_transforms[0].executor);
        assert_eq!(
            recorded.query_transforms[0].litellm_params.get("timeout"),
            Some(&Json::Null)
        );
    }
    {
        // An AWS-signed config signs off the awaiting thread when the caller awaits, and inline when it does not
        let caller_thread = std::thread::current().id();
        for (mode, aws, off_loop) in [
            (ClientKind::Asynchronous, true, true),
            (ClientKind::Asynchronous, false, false),
            (ClientKind::Sync, true, false),
        ] {
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
                .search(&deps, &ThreadedSeam, mode, &call)
                .await
                .expect("the search succeeds");
            let recorded = recorder.lock().unwrap_or_else(PoisonError::into_inner);
            assert_eq!(recorded.signs.len(), 1);
            let signed_on = recorded.threads[0];
            drop(recorded);
            if off_loop {
                assert_ne!(signed_on, caller_thread);
            } else {
                assert_eq!(signed_on, caller_thread);
            }
            assert_eq!(wire.requests().len(), 1);
        }
    }
}

#[tokio::test]
async fn direct_search_bypasses_http() {
    let litellm_params = params(r#"{"api_key":"store-key","timeout":99}"#);
    let optional_params = params(r#"{"max_num_results":2}"#);
    let query = SearchQuery::Text("a query".to_owned());
    let embeddings = Embeddings;
    // A direct store is logged with its synthetic endpoint, and the store gets the timeout and the executor
    for mode in [ClientKind::Sync, ClientKind::Asynchronous] {
        let events = Events::new();
        let store = DirectStore::new(false);
        let call = DirectCall {
            provider: "valkey",
            vector_store_id: "store-a",
            query: &query,
            optional_params: &optional_params,
            litellm_params: &litellm_params,
            timeout: Some(Duration::from_secs(7)),
            embedding_executor: Some(&embeddings),
        };
        let mut fixture = Fixture::new("vector_store_search", &events);
        let answer = match mode {
            ClientKind::Sync => fixture.execute_direct(&store, &call),
            ClientKind::Asynchronous => fixture.aexecute_direct(&store, &call).await,
        }
        .expect("the direct search succeeds");
        let returned: Json =
            serde_json::from_slice(&serde_json::to_vec(&answer).expect("the response encodes"))
                .expect("the response is JSON");
        assert_eq!(returned, json(SEARCH_RESPONSE));
        {
            let observed = store.observed();
            assert_eq!(observed.ids, ["store-a"]);
            assert_eq!(observed.timeouts, [Some(Duration::from_secs(7))]);
            assert_eq!(observed.executors, [true]);
            assert_eq!(observed.litellm_params, [litellm_params.clone()]);
        }
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
        assert_eq!(events.recorded(), ["log"]);
    }
    {
        // A failing store's own error travels unchanged through the sync entry, never as an HTTP-derived provider error
        let events = Events::new();
        let store = DirectStore::new(true);
        let call = DirectCall {
            provider: "valkey",
            vector_store_id: "store-a",
            query: &query,
            optional_params: &optional_params,
            litellm_params: &litellm_params,
            timeout: None,
            embedding_executor: None,
        };
        let mut fixture = Fixture::new("vector_store_search", &events);
        let Err(SearchError::Direct(error)) = fixture.execute_direct(&store, &call) else {
            panic!("the store's failure is passed through");
        };
        assert_eq!(error.message, "the store refused");
        assert_eq!(events.recorded(), ["log"]);
    }
    {
        // The async entry passes the same failure through
        let events = Events::new();
        let store = DirectStore::new(true);
        let call = DirectCall {
            provider: "valkey",
            vector_store_id: "store-a",
            query: &query,
            optional_params: &optional_params,
            litellm_params: &litellm_params,
            timeout: None,
            embedding_executor: None,
        };
        let mut fixture = Fixture::new("vector_store_search", &events);
        let Err(SearchError::Direct(error)) = fixture.aexecute_direct(&store, &call).await else {
            panic!("the store's failure is passed through");
        };
        assert_eq!(error.message, "the store refused");
    }
}

#[tokio::test]
async fn search_timeout_and_errors() {
    let query = SearchQuery::Text("a".to_owned());
    let optional_params = params("{}");
    // A caller timeout becomes the handler's own 408, with its message, no headers and no chained cause
    for mode in [ClientKind::Sync, ClientKind::Asynchronous] {
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
        let Err(SearchError::Provider(error)) =
            fixture.search(&deps, &ThreadedSeam, mode, &call).await
        else {
            panic!("a timeout is the handler's own provider error");
        };
        assert_eq!(error.status_code, 408);
        assert!(!error.status_code_is_synthesized);
        assert_eq!(error.message, TIMEOUT_MESSAGE);
        assert!(error.headers.is_empty());
        assert!(std::error::Error::source(&error).is_none());
    }
    {
        // A status error keeps the response's status, text and masked headers, and is not retried
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
        let Err(SearchError::Provider(error)) = fixture
            .search(&deps, &ThreadedSeam, ClientKind::Sync, &call)
            .await
        else {
            panic!("a 429 is a provider error");
        };
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
        assert_eq!(wire.requests().len(), 1);
    }
    {
        // A transport failure without a response becomes a synthesized 500 after one send
        let events = Events::new();
        let wire = Wire::failing(TransportError::Closed, &events);
        let source = Source::new(wire.clone());
        let deps = SearchDeps {
            config: Arc::new(openai_search_config()),
            source: &source,
            supplied: None,
        };
        let litellm_params = params(r#"{"api_base":"http://mock.test/v1"}"#);
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
    {
        // A response the transform rejects fails with the response's own status, outside the send's error conversion
        let events = Events::new();
        let wire = Wire::replying(Reply::json(200, "\"slow\""), &events);
        let source = Source::new(wire.clone());
        let deps = SearchDeps {
            config: Arc::new(openai_search_config()),
            source: &source,
            supplied: None,
        };
        let litellm_params = params(r#"{"api_base":"http://mock.test/v1"}"#);
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
}
