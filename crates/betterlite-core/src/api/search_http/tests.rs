#![cfg(test)]

mod step_order;
mod support;

use std::collections::BTreeMap;
use std::io::Read as _;
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use betterlite_config::tls::TlsPolicy;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::pool::ClientPool;
use betterlite_http_fixture::error::FixtureError;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::{Serving, serve};
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::provider::{HttpMethod, SearchQuery};
use betterlite_openai::search::records::SearchResponse;
use betterlite_test_ports::full_listener::FullListener;
use betterlite_test_ports::refused_port::RefusedPort;
use tokio::time::sleep;

use self::support::{
    Fixture, TestError, TestProvider, default_tls, provider_error, proxied, proxy_policy,
    recorded_errors, search_call, test_provider,
};
use super::{SearchHttpCall, SearchHttpError};

const ANSWER: &str = r#"{"results":[{"title":"t","url":"u","snippet":"s"}]}"#;
const NOT_AN_ANSWER: &str = "not a search response";

#[tokio::test]
async fn post_body_sent() {
    let served = serve(reply("200 OK", &[], ANSWER), Duration::ZERO).expect("the fixture serves");
    let base = format!("http://{}/search", served.address);
    let tls = default_tls();
    let routing = proxy_policy();
    let pool = ClientPool::new(4, TransportLimits::DEFAULT);
    let provider = test_provider(&base);
    let query = SearchQuery::Single(String::from("rust"));
    let params = Object::new();
    let mut fixture = Fixture::new();
    let response = fixture
        .execute(
            search_call(&query, &params, &base, &tls, &routing),
            &provider,
            &pool,
            false,
        )
        .await
        .expect("the search succeeds");
    assert_eq!(
        response,
        serde_json::from_str::<SearchResponse>(ANSWER).expect("the answer fixture")
    );
    let request = served.finish().expect("the fixture answers");
    assert_eq!(request.line, "POST /search HTTP/1.1");
    assert!(
        request
            .head
            .to_lowercase()
            .contains("content-type: application/json")
    );
    assert_eq!(request.body, r#"{"q":"rust"}"#);
}

#[tokio::test]
async fn get_without_body() {
    let served = serve(reply("200 OK", &[], ANSWER), Duration::ZERO).expect("the fixture serves");
    let base = format!("http://{}/search?q=rust", served.address);
    let tls = default_tls();
    let routing = proxy_policy();
    let pool = ClientPool::new(4, TransportLimits::DEFAULT);
    let provider = TestProvider {
        method: HttpMethod::Get,
        signed: BTreeMap::from([("x-signature".to_owned(), "abc".to_owned())]),
        signed_body: Some(b"{\"signed\":true}".to_vec()),
        ..test_provider(&base)
    };
    let query = SearchQuery::Single(String::from("rust"));
    let params = Object::new();
    let mut fixture = Fixture::new();
    let response = fixture
        .execute(
            search_call(&query, &params, &base, &tls, &routing),
            &provider,
            &pool,
            false,
        )
        .await
        .expect("the search succeeds");
    assert_eq!(response.results.len(), 1);
    let request = served.finish().expect("the fixture answers");
    assert_eq!(request.line, "GET /search?q=rust HTTP/1.1");
    assert_eq!(request.body, "");
    assert!(request.head.to_lowercase().contains("x-signature: abc"));
    assert!(!request.head.to_lowercase().contains("content-type"));
}

#[tokio::test]
async fn signed_body_verbatim() {
    let served = serve(reply("200 OK", &[], ANSWER), Duration::ZERO).expect("the fixture serves");
    let base = format!("http://{}/search", served.address);
    let tls = default_tls();
    let routing = proxy_policy();
    let pool = ClientPool::new(4, TransportLimits::DEFAULT);
    let provider = TestProvider {
        signed: BTreeMap::from([("x-signature".to_owned(), "abc".to_owned())]),
        signed_body: Some(b"{ \"q\" : \"rust\" }".to_vec()),
        ..test_provider(&base)
    };
    let query = SearchQuery::Single(String::from("rust"));
    let params = Object::new();
    let mut fixture = Fixture::new();
    fixture
        .execute(
            search_call(&query, &params, &base, &tls, &routing),
            &provider,
            &pool,
            false,
        )
        .await
        .expect("the search succeeds");
    let request = served.finish().expect("the fixture answers");
    assert_eq!(request.body, "{ \"q\" : \"rust\" }");
    assert!(request.head.to_lowercase().contains("x-signature: abc"));
    assert!(
        request
            .head
            .to_lowercase()
            .contains("content-type: application/json")
    );
}

struct Rejection {
    status: &'static str,
    headers: &'static [(&'static str, &'static str)],
    body: &'static str,
    method: HttpMethod,
    path: &'static str,
}

const TOO_MANY_REQUESTS: Rejection = Rejection {
    status: "429 Too Many Requests",
    headers: &[("retry-after", "3"), ("content-encoding", "identity")],
    body: "slow down",
    method: HttpMethod::Post,
    path: "/search",
};

const SERVICE_UNAVAILABLE: Rejection = Rejection {
    status: "503 Service Unavailable",
    headers: &[("content-encoding", "identity")],
    body: "try later",
    method: HttpMethod::Get,
    path: "/search?q=rust",
};

async fn reject_with(spec: &Rejection) -> (SearchHttpError<TestError>, TestProvider, Serving) {
    let served = serve(reply(spec.status, spec.headers, spec.body), Duration::ZERO)
        .expect("the fixture serves");
    let base = format!("http://{}{}", served.address, spec.path);
    let tls = default_tls();
    let routing = proxy_policy();
    let pool = ClientPool::new(4, TransportLimits::DEFAULT);
    let provider = TestProvider {
        method: spec.method,
        ..test_provider(&base)
    };
    let query = SearchQuery::Single(String::from("rust"));
    let params = Object::new();
    let mut fixture = Fixture::new();
    let rejection = fixture
        .execute(
            search_call(&query, &params, &base, &tls, &routing),
            &provider,
            &pool,
            false,
        )
        .await
        .expect_err("the status fails the call");
    (rejection, provider, served)
}

async fn status_error_is_a_provider_error() {
    let (rejection, provider, served) = reject_with(&TOO_MANY_REQUESTS).await;
    let error = provider_error(&rejection);
    assert_eq!(
        (error.status_code, error.message.as_str()),
        (429, "slow down")
    );
    assert!(!error.status_code_is_synthesized);
    let headers = error
        .headers
        .clone()
        .expect("the provider error keeps the reply headers");
    assert!(headers.contains(&("retry-after".to_owned(), "3".to_owned())));
    let seen_errors = recorded_errors(&provider);
    assert_eq!(seen_errors.len(), 1);
    let (status, text, sent_headers) = seen_errors.first().expect("one http_error call");
    assert_eq!((*status, text.as_str()), (429, "slow down"));
    assert_eq!(
        sent_headers.get("retry-after").map(String::as_str),
        Some("3")
    );
    assert_eq!(
        sent_headers.get("content-length").map(String::as_str),
        Some("9")
    );
    assert!(!sent_headers.contains_key("content-encoding"));
    assert_eq!(
        served.finish().expect("the fixture answers").line,
        "POST /search HTTP/1.1"
    );
}

async fn get_status_error_is_a_provider_error() {
    let (rejection, provider, served) = reject_with(&SERVICE_UNAVAILABLE).await;
    let error = provider_error(&rejection);
    assert_eq!(
        (error.status_code, error.status_code_is_synthesized),
        (503, false)
    );
    assert_eq!(error.message, "try later");
    let errors = recorded_errors(&provider);
    let (_, _, sent_headers) = errors.first().expect("one http_error call");
    assert_eq!(
        sent_headers.get("content-encoding").map(String::as_str),
        Some("identity")
    );
    assert_eq!(
        sent_headers.get("content-length").map(String::as_str),
        Some("9")
    );
    assert_eq!(
        served.finish().expect("the fixture answers").line,
        "GET /search?q=rust HTTP/1.1"
    );
}

async fn transform_error_is_not_converted() {
    let served =
        serve(reply("200 OK", &[], NOT_AN_ANSWER), Duration::ZERO).expect("the fixture serves");
    let base = format!("http://{}/search", served.address);
    let tls = default_tls();
    let routing = proxy_policy();
    let pool = ClientPool::new(4, TransportLimits::DEFAULT);
    let provider = TestProvider {
        reject: true,
        ..test_provider(&base)
    };
    let query = SearchQuery::Single(String::from("rust"));
    let params = Object::new();
    let mut fixture = Fixture::new();
    let rejection = fixture
        .execute(
            search_call(&query, &params, &base, &tls, &routing),
            &provider,
            &pool,
            false,
        )
        .await
        .expect_err("the reply fails the call");
    match &rejection {
        SearchHttpError::Hook(TestError::Rejected(text)) => assert_eq!(text, NOT_AN_ANSWER),
        SearchHttpError::Hook(other) => panic!("the reply failed another way: {other}"),
        SearchHttpError::Provider { provider: error } => {
            panic!("the reply reached the converter: {}", error.message);
        }
        SearchHttpError::Logging(logging) => panic!("the reply reached logging: {logging}"),
    }
    assert!(recorded_errors(&provider).is_empty());
    assert_eq!(
        served.finish().expect("the fixture answers").line,
        "POST /search HTTP/1.1"
    );
}

#[tokio::test]
async fn status_failure() {
    status_error_is_a_provider_error().await;
    get_status_error_is_a_provider_error().await;
    transform_error_is_not_converted().await;
    repeated_headers_are_joined().await;
}

async fn repeated_headers_are_joined() {
    let repeated = [("warning", "199 first"), ("warning", "299 second")];
    let served = serve(
        reply("429 Too Many Requests", &repeated, "slow down"),
        Duration::ZERO,
    )
    .expect("the fixture serves");
    let base = format!("http://{}/search", served.address);
    let tls = default_tls();
    let routing = proxy_policy();
    let pool = ClientPool::new(4, TransportLimits::DEFAULT);
    let provider = test_provider(&base);
    let query = SearchQuery::Single(String::from("rust"));
    let params = Object::new();
    let mut fixture = Fixture::new();
    let rejection = fixture
        .execute(
            search_call(&query, &params, &base, &tls, &routing),
            &provider,
            &pool,
            false,
        )
        .await
        .expect_err("the reply fails the call");
    let error = provider_error(&rejection);
    assert_eq!(error.status_code, 429);
    assert_eq!(
        error
            .headers
            .as_ref()
            .and_then(|headers| headers.get("warning"))
            .map(String::as_str),
        Some("199 first, 299 second")
    );
    served.finish().expect("the fixture answers");
}

async fn connection_failure_is_converted() {
    let refused = RefusedPort::bind().expect("the refused port binds");
    let base = format!("http://{}/search", refused.address);
    let tls = default_tls();
    let routing = proxy_policy();
    let pool = ClientPool::new(4, TransportLimits::DEFAULT);
    let provider = test_provider(&base);
    let query = SearchQuery::Single(String::from("rust"));
    let params = Object::new();
    let mut fixture = Fixture::new();
    let rejection = fixture
        .execute(
            search_call(&query, &params, &base, &tls, &routing),
            &provider,
            &pool,
            false,
        )
        .await
        .expect_err("the connection fails");
    let error = provider_error(&rejection);
    assert_eq!(
        (error.status_code, error.status_code_is_synthesized),
        (500, true)
    );
    assert_eq!(error.message, "the connection could not be made");
    assert!(recorded_errors(&provider).is_empty());
}

async fn deadline_failure_is_a_timeout() {
    let served = serve(reply("200 OK", &[], ANSWER), Duration::from_millis(300))
        .expect("the fixture serves");
    let base = format!("http://{}/search", served.address);
    let tls = default_tls();
    let routing = proxy_policy();
    let pool = ClientPool::new(4, TransportLimits::DEFAULT);
    let provider = test_provider(&base);
    let query = SearchQuery::Single(String::from("rust"));
    let params = Object::new();
    let mut fixture = Fixture::new();
    let call = SearchHttpCall {
        timeout: Duration::from_millis(50),
        ..search_call(&query, &params, &base, &tls, &routing)
    };
    let rejection = fixture
        .execute(call, &provider, &pool, false)
        .await
        .expect_err("the deadline fails the call");
    let error = provider_error(&rejection);
    assert_eq!(
        (error.status_code, error.status_code_is_synthesized),
        (408, false)
    );
    assert!(
        error
            .message
            .starts_with("litellm.Timeout: Connection timed out. Timeout passed=0.05, time taken=")
    );
    assert!(recorded_errors(&provider).is_empty());
    served.finish().expect("the fixture answers");
}

async fn closed_pool_is_converted() {
    let tls = default_tls();
    let routing = proxy_policy();
    let pool = ClientPool::new(4, TransportLimits::DEFAULT);
    let drained = pool
        .client(&tls, &routing)
        .expect("the pool builds a client");
    drained
        .drain(Duration::from_secs(1))
        .await
        .expect("the client drains");
    let other_tls = TlsPolicy {
        user_agent: String::from("other"),
        ..tls.clone()
    };
    let base = String::from("http://127.0.0.1:1/search");
    let provider = test_provider(&base);
    let query = SearchQuery::Single(String::from("rust"));
    let params = Object::new();
    let mut fixture = Fixture::new();
    let call = SearchHttpCall {
        tls: &other_tls,
        ..search_call(&query, &params, &base, &tls, &routing)
    };
    let rejection = fixture
        .execute(call, &provider, &pool, false)
        .await
        .expect_err("the pool is closed");
    let error = provider_error(&rejection);
    assert_eq!(
        (error.status_code, error.status_code_is_synthesized),
        (500, true)
    );
    assert_eq!(error.message, "the pool is closed");
    assert!(recorded_errors(&provider).is_empty());
}

async fn body_too_large_is_converted() {
    let served = serve(reply("200 OK", &[], ANSWER), Duration::ZERO).expect("the fixture serves");
    let base = format!("http://{}/search", served.address);
    let tls = default_tls();
    let routing = proxy_policy();
    let pool = ClientPool::new(4, TransportLimits::DEFAULT);
    let provider = test_provider(&base);
    let query = SearchQuery::Single(String::from("rust"));
    let params = Object::new();
    let mut fixture = Fixture::new();
    let call = SearchHttpCall {
        client: Some(proxied(
            TransportLimits {
                body_bytes: 5,
                ..TransportLimits::DEFAULT
            },
            &routing,
        )),
        ..search_call(&query, &params, &base, &tls, &routing)
    };
    let rejection = fixture
        .execute(call, &provider, &pool, false)
        .await
        .expect_err("the body is too large");
    let error = provider_error(&rejection);
    assert_eq!(
        (error.status_code, error.status_code_is_synthesized),
        (500, true)
    );
    assert_eq!(error.message, "the response body is larger than 5 bytes");
    assert!(recorded_errors(&provider).is_empty());
    served.finish().expect("the fixture answers");
}

async fn request_failure(provider: &TestProvider, expected: &str) {
    let tls = default_tls();
    let routing = proxy_policy();
    let pool = ClientPool::new(4, TransportLimits::DEFAULT);
    let base = provider.complete.clone();
    let query = SearchQuery::Single(String::from("rust"));
    let params = Object::new();
    let mut fixture = Fixture::new();
    let rejection = fixture
        .execute(
            search_call(&query, &params, &base, &tls, &routing),
            provider,
            &pool,
            false,
        )
        .await
        .expect_err("the request cannot be built");
    let error = provider_error(&rejection);
    assert_eq!(
        (error.status_code, error.status_code_is_synthesized),
        (500, true)
    );
    assert_eq!(error.message, expected);
    assert!(recorded_errors(provider).is_empty());
}

async fn request_failure_is_converted() {
    let no_url = String::from("not a url");
    let url_provider = test_provider(&no_url);
    request_failure(
        &url_provider,
        "the search request URL not a url is not a URL",
    )
    .await;
    let base = String::from("http://127.0.0.1:1/search");
    let bad_name = TestProvider {
        signed: BTreeMap::from([(String::from("bad header"), String::from("1"))]),
        ..test_provider(&base)
    };
    request_failure(&bad_name, "the bad header header name is not a header name").await;
    let bad_value = TestProvider {
        signed: BTreeMap::from([(String::from("x-signature"), String::from("bad\nvalue"))]),
        ..test_provider(&base)
    };
    request_failure(
        &bad_value,
        "the x-signature header value is not a header value",
    )
    .await;
}

fn dropping_listener() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("the listener binds");
    let address = listener.local_addr().expect("the listener address reads");
    let attempts = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&attempts);
    thread::Builder::new()
        .spawn(move || {
            for _ in 0..2_u8 {
                let Ok((mut stream, _peer)) = listener.accept() else {
                    return;
                };
                seen.fetch_add(1, Ordering::SeqCst);
                let mut buffer = [0_u8; 1024];
                let _read = stream.read(&mut buffer);
            }
        })
        .expect("the listener thread starts");
    (format!("http://{address}/search"), attempts)
}

async fn dropped_connection_is_not_retried() {
    let (base, attempts) = dropping_listener();
    let tls = default_tls();
    let routing = proxy_policy();
    let pool = ClientPool::new(4, TransportLimits::DEFAULT);
    let provider = test_provider(&base);
    let query = SearchQuery::Single(String::from("rust"));
    let params = Object::new();
    let mut fixture = Fixture::new();
    let rejection = fixture
        .execute(
            search_call(&query, &params, &base, &tls, &routing),
            &provider,
            &pool,
            false,
        )
        .await
        .expect_err("the dropped connection fails the call");
    let error = provider_error(&rejection);
    assert_eq!(
        (error.status_code, error.status_code_is_synthesized),
        (500, true)
    );
    sleep(Duration::from_millis(100)).await;
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn transport_failure() {
    connection_failure_is_converted().await;
    deadline_failure_is_a_timeout().await;
    connect_timeout_is_a_timeout().await;
    closed_pool_is_converted().await;
    body_too_large_is_converted().await;
    request_failure_is_converted().await;
    dropped_connection_is_not_retried().await;
}

async fn connect_timeout_is_a_timeout() {
    let full = FullListener::bind().expect("the full listener binds");
    let base = format!("http://{}/search", full.address);
    let tls = default_tls();
    let routing = proxy_policy();
    let pool = ClientPool::new(4, TransportLimits::DEFAULT);
    let provider = test_provider(&base);
    let query = SearchQuery::Single(String::from("rust"));
    let params = Object::new();
    let mut fixture = Fixture::new();
    let call = SearchHttpCall {
        client: Some(proxied(
            TransportLimits {
                connect_timeout: Duration::from_millis(50),
                ..TransportLimits::DEFAULT
            },
            &routing,
        )),
        ..search_call(&query, &params, &base, &tls, &routing)
    };
    let rejection = fixture
        .execute(call, &provider, &pool, false)
        .await
        .expect_err("the connect timeout fails the call");
    let error = provider_error(&rejection);
    assert_eq!(
        (error.status_code, error.status_code_is_synthesized),
        (408, false)
    );
    assert!(
        error
            .message
            .starts_with("litellm.Timeout: Connection timed out. Timeout passed=2, time taken=")
    );
    assert!(recorded_errors(&provider).is_empty());
}

async fn supplied_client_wins() {
    let served = serve(reply("200 OK", &[], ANSWER), Duration::ZERO).expect("the fixture serves");
    let base = format!("http://{}/search", served.address);
    let tls = default_tls();
    let routing = proxy_policy();
    let pool = ClientPool::new(4, TransportLimits::DEFAULT);
    let supplied = proxied(TransportLimits::DEFAULT, &routing);
    supplied
        .drain(Duration::from_secs(1))
        .await
        .expect("the supplied client drains");
    let provider = test_provider(&base);
    let query = SearchQuery::Single(String::from("rust"));
    let params = Object::new();
    let mut fixture = Fixture::new();
    let call = SearchHttpCall {
        client: Some(supplied),
        ..search_call(&query, &params, &base, &tls, &routing)
    };
    let rejection = fixture
        .execute(call, &provider, &pool, false)
        .await
        .expect_err("the supplied client is used");
    assert_eq!(
        provider_error(&rejection).message,
        "the client is shutting down and takes no new requests"
    );
    assert!(matches!(served.finish(), Err(FixtureError::NoRequest)));
    assert_eq!(pool.client_count(), 0);
}

async fn pool_client_is_used_when_none_is_supplied() {
    let served = serve(reply("200 OK", &[], ANSWER), Duration::ZERO).expect("the fixture serves");
    let base = format!("http://{}/search", served.address);
    let tls = default_tls();
    let routing = proxy_policy();
    let pool = ClientPool::new(4, TransportLimits::DEFAULT);
    let provider = test_provider(&base);
    let query = SearchQuery::Single(String::from("rust"));
    let params = Object::new();
    let mut fixture = Fixture::new();
    let response = fixture
        .execute(
            search_call(&query, &params, &base, &tls, &routing),
            &provider,
            &pool,
            false,
        )
        .await
        .expect("the pool client sends");
    assert_eq!(response.results.len(), 1);
    assert_eq!(
        served.finish().expect("the fixture answers").line,
        "POST /search HTTP/1.1"
    );
    assert_eq!(pool.client_count(), 1);
}

async fn network_mock_answers_before_the_pool() {
    let served = serve(reply("200 OK", &[], ANSWER), Duration::ZERO).expect("the fixture serves");
    let base = format!("http://{}/search", served.address);
    let tls = default_tls();
    let routing = proxy_policy();
    let pool = ClientPool::new(4, TransportLimits::DEFAULT);
    let provider = TestProvider {
        answer: Some(serde_json::from_str::<SearchResponse>(ANSWER).expect("the answer fixture")),
        ..test_provider(&base)
    };
    let query = SearchQuery::Single(String::from("rust"));
    let params = Object::new();
    let mut fixture = Fixture::new();
    let response = fixture
        .execute(
            search_call(&query, &params, &base, &tls, &routing),
            &provider,
            &pool,
            true,
        )
        .await
        .expect("the mock answers");
    assert_eq!(response.results.len(), 1);
    assert!(matches!(served.finish(), Err(FixtureError::NoRequest)));
}

async fn post_uses_the_call_timeout() {
    let served = serve(reply("200 OK", &[], ANSWER), Duration::from_millis(300))
        .expect("the fixture serves");
    let base = format!("http://{}/search", served.address);
    let tls = default_tls();
    let routing = proxy_policy();
    let pool = ClientPool::new(4, TransportLimits::DEFAULT);
    let provider = test_provider(&base);
    let query = SearchQuery::Single(String::from("rust"));
    let params = Object::new();
    let mut fixture = Fixture::new();
    let call = SearchHttpCall {
        timeout: Duration::from_millis(50),
        ..search_call(&query, &params, &base, &tls, &routing)
    };
    let rejection = fixture
        .execute(call, &provider, &pool, false)
        .await
        .expect_err("the call timeout is the POST deadline");
    assert_eq!(provider_error(&rejection).status_code, 408);
    served.finish().expect("the fixture answers");
}

async fn get_uses_the_client_timeout() {
    let served = serve(reply("200 OK", &[], ANSWER), Duration::from_millis(300))
        .expect("the fixture serves");
    let base = format!("http://{}/search?q=rust", served.address);
    let tls = default_tls();
    let routing = proxy_policy();
    let pool = ClientPool::new(4, TransportLimits::DEFAULT);
    let provider = TestProvider {
        method: HttpMethod::Get,
        ..test_provider(&base)
    };
    let supplied = proxied(TransportLimits::DEFAULT, &routing);
    let query = SearchQuery::Single(String::from("rust"));
    let params = Object::new();
    let mut fixture = Fixture::new();
    let call = SearchHttpCall {
        client: Some(supplied),
        timeout: Duration::from_millis(50),
        ..search_call(&query, &params, &base, &tls, &routing)
    };
    let response = fixture
        .execute(call, &provider, &pool, false)
        .await
        .expect("the client's own deadline applies to GET");
    assert_eq!(response.results.len(), 1);
    assert_eq!(
        served.finish().expect("the fixture answers").line,
        "GET /search?q=rust HTTP/1.1"
    );
}

#[tokio::test]
async fn client_selection() {
    supplied_client_wins().await;
    pool_client_is_used_when_none_is_supplied().await;
    network_mock_answers_before_the_pool().await;
    post_uses_the_call_timeout().await;
    get_uses_the_client_timeout().await;
}
