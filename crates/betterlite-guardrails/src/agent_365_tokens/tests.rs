#![cfg(test)]

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_http::proxy::base_builder;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::poster::{Agent365Poster, Agent365PosterError, HttpPoster, PostReply};
use super::{
    Agent365Guardrail, Agent365Settings, DEFAULT_TIMEOUT_SECONDS, DEFENDER_STATUS_EVALUATED,
    EVALUATE_URL, SUPPORTED_EVENT_HOOKS,
};
use crate::agent_365_config::{
    AGENT_365_PROD_API_BASE, AGENT_365_PROD_RESOURCE_APP_ID, AGENT_365_SCOPE_NAME,
};
use crate::agent_365_tokens::cache::{OBO_CACHE_MAX_ENTRIES, TOKEN_EXPIRY_MARGIN};
use crate::agent_365_tokens::error::{
    Agent365Error, Agent365TokenError, Agent365TokenExchangeError,
};
use crate::agent_365_tokens::exchange::{OBO_SCOPE, entra_assertion};
use crate::agent_365_tokens::wire::{DEFAULT_TOKEN_TTL, MAX_DESCRIPTION_CHARS};

type ScenarioFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
type Scenario = fn() -> ScenarioFuture;

const START_SECONDS: u64 = 1_000_000;
const ASSERTION: &str = "header.payload.signature";

#[derive(Clone)]
struct TestClock {
    now: Arc<AtomicU64>,
}

impl TestClock {
    fn new() -> Self {
        Self {
            now: Arc::new(AtomicU64::new(START_SECONDS)),
        }
    }

    fn advance(&self, seconds: u64) {
        let _previous = self.now.fetch_add(seconds, Ordering::Relaxed);
    }

    fn handle(&self) -> Arc<dyn Fn() -> SystemTime + Send + Sync> {
        let now = Arc::clone(&self.now);
        Arc::new(move || {
            UNIX_EPOCH
                .checked_add(Duration::from_secs(now.load(Ordering::Relaxed)))
                .unwrap()
        })
    }
}

fn start_time() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(START_SECONDS))
        .unwrap()
}

fn settings() -> Agent365Settings {
    Agent365Settings {
        guardrail_name: "agent365".to_owned(),
        tenant_id: "tenant-1".to_owned(),
        client_id: "client-1".to_owned(),
        client_secret: SecretString::from("secret-1".to_owned()),
        request_timeout: None,
        unreachable_fallback: UnreachableFallback::FailClosed,
        event_hook: None,
        default_on: false,
    }
}

fn reply(status: u16, body: &str) -> PostReply {
    PostReply {
        status: StatusCode::from_u16(status).unwrap(),
        body: body.as_bytes().to_vec(),
    }
}

#[derive(Clone, Debug)]
struct RecordedRequest {
    url: String,
    content_type: Option<String>,
    body: String,
    timeout: Duration,
}

#[derive(Default)]
struct FakePoster {
    replies: Mutex<VecDeque<PostReply>>,
    seen: Mutex<Vec<RecordedRequest>>,
}

impl FakePoster {
    fn queue(&self, replies: Vec<PostReply>) {
        self.replies.lock().unwrap().extend(replies);
    }

    fn recorded(&self) -> Vec<RecordedRequest> {
        self.seen.lock().unwrap().clone()
    }

    fn request_count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}

#[async_trait]
impl Agent365Poster for FakePoster {
    async fn post(
        &self,
        url: &str,
        headers: &HeaderMap,
        body: &[u8],
        timeout: Duration,
    ) -> Result<PostReply, Agent365PosterError> {
        let content_type = headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        self.seen.lock().unwrap().push(RecordedRequest {
            url: url.to_owned(),
            content_type,
            body: String::from_utf8(body.to_vec()).unwrap(),
            timeout,
        });
        Ok(self.replies.lock().unwrap().pop_front().unwrap())
    }
}

fn guardrail(poster: Arc<FakePoster>) -> (Agent365Guardrail, TestClock) {
    let clock = TestClock::new();
    let guardrail = Agent365Guardrail::new(settings(), poster, true, clock.handle()).unwrap();
    (guardrail, clock)
}

fn cache_key(assertion: &str) -> String {
    hex::encode(Sha256::digest(assertion.as_bytes()))
}

fn cached_tokens(guardrail: &Agent365Guardrail) -> Vec<(String, String)> {
    let cache = guardrail.cache.lock().unwrap();
    cache
        .iter()
        .rev()
        .map(|(key, entry)| (key.clone(), entry.access_token.expose_secret().to_owned()))
        .collect()
}

fn cached_keys(guardrail: &Agent365Guardrail) -> Vec<String> {
    cached_tokens(guardrail)
        .into_iter()
        .map(|(key, _token)| key)
        .collect()
}

fn expiry_offset(guardrail: &Agent365Guardrail, assertion: &str) -> Duration {
    let expires_at = guardrail
        .cache
        .lock()
        .unwrap()
        .peek(&cache_key(assertion))
        .unwrap()
        .expires_at;
    expires_at.duration_since(start_time()).unwrap()
}

async fn fetch(guardrail: &Agent365Guardrail, assertion: &str) -> String {
    guardrail
        .get_obo_token(assertion)
        .await
        .unwrap()
        .expose_secret()
        .to_owned()
}

async fn fetch_many(guardrail: &Agent365Guardrail, count: usize) {
    for index in 0..count {
        let assertion = format!("assertion-{index}");
        let _token = fetch(guardrail, &assertion).await;
    }
}

fn queue_series(poster: &FakePoster, count: usize) {
    let replies: Vec<PostReply> = (0..count)
        .map(|index| {
            reply(
                200,
                &format!(r#"{{"access_token":"tok-{index}","expires_in":3600}}"#),
            )
        })
        .collect();
    poster.queue(replies);
}

fn token_error(result: Result<SecretString, Agent365TokenError>) -> Agent365TokenError {
    result.unwrap_err()
}

fn exchange_failure(
    result: Result<SecretString, Agent365TokenError>,
) -> Agent365TokenExchangeError {
    let Agent365TokenError::Exchange(failure) = result.unwrap_err() else {
        panic!("expected a token exchange failure");
    };
    failure
}

fn test_token_is_cached_per_assertion() -> ScenarioFuture {
    Box::pin(async {
        let poster = Arc::new(FakePoster::default());
        poster.queue(vec![reply(
            200,
            r#"{"access_token":"tok-1","expires_in":3600}"#,
        )]);
        let (guardrail, _clock) = guardrail(Arc::clone(&poster));

        let first = fetch(&guardrail, ASSERTION).await;
        let second = fetch(&guardrail, ASSERTION).await;

        assert_eq!(first, "tok-1");
        assert_eq!(second, "tok-1");
        assert_eq!(poster.request_count(), 1);
        let recorded = poster.recorded();
        let request = recorded.first().unwrap();
        assert_eq!(
            request.url,
            "https://login.microsoftonline.com/tenant-1/oauth2/v2.0/token"
        );
        assert_eq!(
            request.content_type.as_deref(),
            Some("application/x-www-form-urlencoded")
        );
        assert_eq!(
            request.body,
            "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer&client_id=client-1&client_secret=secret-1&assertion=header.payload.signature&scope=ea9ffc3e-8a23-4a7d-836d-234d7c7565c1%2FThreatProtection.Evaluate.All&requested_token_use=on_behalf_of"
        );
        assert_eq!(request.timeout, Duration::from_secs(10));
        assert_eq!(cached_keys(&guardrail), vec![cache_key(ASSERTION)]);
        assert_eq!(cached_tokens(&guardrail).first().unwrap().1, "tok-1");
    })
}

fn test_token_within_the_margin_is_reused() -> ScenarioFuture {
    Box::pin(async {
        let poster = Arc::new(FakePoster::default());
        poster.queue(vec![
            reply(200, r#"{"access_token":"tok-1","expires_in":62}"#),
            reply(200, r#"{"access_token":"tok-2","expires_in":62}"#),
        ]);
        let (guardrail, clock) = guardrail(Arc::clone(&poster));

        let first = fetch(&guardrail, ASSERTION).await;
        clock.advance(1);
        let second = fetch(&guardrail, ASSERTION).await;

        assert_eq!(first, "tok-1");
        assert_eq!(second, "tok-1");
        assert_eq!(poster.request_count(), 1);
    })
}

fn test_token_at_the_margin_is_refetched() -> ScenarioFuture {
    Box::pin(async {
        let poster = Arc::new(FakePoster::default());
        poster.queue(vec![
            reply(200, r#"{"access_token":"tok-1","expires_in":61}"#),
            reply(200, r#"{"access_token":"tok-2","expires_in":61}"#),
        ]);
        let (guardrail, clock) = guardrail(Arc::clone(&poster));

        let first = fetch(&guardrail, ASSERTION).await;
        clock.advance(1);
        let second = fetch(&guardrail, ASSERTION).await;

        assert_eq!(first, "tok-1");
        assert_eq!(second, "tok-2");
        assert_eq!(poster.request_count(), 2);
    })
}

fn test_expired_token_is_refetched() -> ScenarioFuture {
    Box::pin(async {
        let poster = Arc::new(FakePoster::default());
        poster.queue(vec![
            reply(200, r#"{"access_token":"tok-1","expires_in":60}"#),
            reply(200, r#"{"access_token":"tok-2","expires_in":60}"#),
        ]);
        let (guardrail, _clock) = guardrail(Arc::clone(&poster));

        let first = fetch(&guardrail, ASSERTION).await;
        let second = fetch(&guardrail, ASSERTION).await;

        assert_eq!(first, "tok-1");
        assert_eq!(second, "tok-2");
        assert_eq!(poster.request_count(), 2);
    })
}

fn test_expires_in_forms_are_parsed() -> ScenarioFuture {
    Box::pin(async {
        let forms = [
            ("120", Duration::from_secs(120)),
            ("\"120\"", Duration::from_secs(120)),
            ("\" 120 \"", Duration::from_secs(120)),
            ("120.5", Duration::from_millis(120_500)),
        ];
        for (ttl, expected) in forms {
            let poster = Arc::new(FakePoster::default());
            let body = format!(r#"{{"access_token":"tok-1","expires_in":{ttl}}}"#);
            poster.queue(vec![reply(200, &body)]);
            let (guardrail, _clock) = guardrail(Arc::clone(&poster));
            let _token = fetch(&guardrail, ASSERTION).await;
            assert_eq!(expiry_offset(&guardrail, ASSERTION), expected);
        }
    })
}

fn test_default_ttl_applies_until_the_margin() -> ScenarioFuture {
    Box::pin(async {
        for body in [
            r#"{"access_token":"tok-1","expires_in":"soon"}"#,
            r#"{"access_token":"tok-1","expires_in":null}"#,
            r#"{"access_token":"tok-1"}"#,
            r#"{"access_token":"tok-1","expires_in":-5}"#,
            r#"{"access_token":"tok-1","expires_in":"inf"}"#,
        ] {
            let poster = Arc::new(FakePoster::default());
            poster.queue(vec![reply(200, body), reply(200, body)]);
            let (guardrail, clock) = guardrail(Arc::clone(&poster));
            let _first = fetch(&guardrail, ASSERTION).await;
            assert_eq!(expiry_offset(&guardrail, ASSERTION), DEFAULT_TOKEN_TTL);
            clock.advance(3538);
            let _cached = fetch(&guardrail, ASSERTION).await;
            assert_eq!(poster.request_count(), 1);
            clock.advance(1);
            let _refetched = fetch(&guardrail, ASSERTION).await;
            assert_eq!(poster.request_count(), 2);
        }
    })
}

fn test_throttled_and_server_errors() -> ScenarioFuture {
    Box::pin(async {
        let poster = Arc::new(FakePoster::default());
        poster.queue(vec![
            reply(408, "not json"),
            reply(429, "{}"),
            reply(500, "not json"),
            reply(503, "{}"),
        ]);
        let (guardrail, _clock) = guardrail(Arc::clone(&poster));

        let throttled = token_error(guardrail.get_obo_token(ASSERTION).await);
        assert!(matches!(
            &throttled,
            Agent365TokenError::Throttled { status: 408 }
        ));
        assert_eq!(throttled.to_string(), "HTTP 408");
        assert!(matches!(
            token_error(guardrail.get_obo_token("second.assertion.value").await),
            Agent365TokenError::Throttled { status: 429 }
        ));
        assert!(matches!(
            token_error(guardrail.get_obo_token("third.assertion.value").await),
            Agent365TokenError::HttpStatus { status: 500 }
        ));
        assert!(matches!(
            token_error(guardrail.get_obo_token("fourth.assertion.value").await),
            Agent365TokenError::HttpStatus { status: 503 }
        ));
        assert!(cached_tokens(&guardrail).is_empty());
    })
}

fn test_lifetime_past_the_clock_range_is_refused() -> ScenarioFuture {
    Box::pin(async {
        let poster = Arc::new(FakePoster::default());
        poster.queue(vec![reply(
            200,
            r#"{"access_token":"tok-1","expires_in":1e19}"#,
        )]);
        let (guardrail, _clock) = guardrail(Arc::clone(&poster));

        assert!(matches!(
            token_error(guardrail.get_obo_token(ASSERTION).await),
            Agent365TokenError::TokenExpiry
        ));
        assert!(cached_tokens(&guardrail).is_empty());
    })
}

fn malformed_variant(error: &Agent365TokenError) -> &'static str {
    match error {
        Agent365TokenError::NonJsonBody { .. } => "non_json_body",
        Agent365TokenError::NonObjectBody { .. } => "non_object_body",
        Agent365TokenError::MissingAccessToken => "missing_access_token",
        Agent365TokenError::BadAccessToken => "bad_access_token",
        Agent365TokenError::Exchange(_)
        | Agent365TokenError::Throttled { .. }
        | Agent365TokenError::HttpStatus { .. }
        | Agent365TokenError::TokenExpiry
        | Agent365TokenError::Poster(_) => "other",
    }
}

fn test_malformed_bodies() -> ScenarioFuture {
    Box::pin(async {
        let poster = Arc::new(FakePoster::default());
        poster.queue(vec![
            reply(200, "not json"),
            reply(200, "[1, 2]"),
            reply(400, "not json"),
            reply(200, r#"{"expires_in":3600}"#),
            reply(200, r#"{"access_token":5}"#),
            reply(200, r#"{"access_token":""}"#),
            reply(200, r#"{"access_token":null}"#),
        ]);
        let (guardrail, _clock) = guardrail(Arc::clone(&poster));
        let cases = [
            ("first.assertion.value", "non_json_body"),
            ("second.assertion.value", "non_object_body"),
            ("third.assertion.value", "non_json_body"),
            ("fourth.assertion.value", "missing_access_token"),
            ("fifth.assertion.value", "bad_access_token"),
            ("sixth.assertion.value", "bad_access_token"),
            ("seventh.assertion.value", "bad_access_token"),
        ];
        for (assertion, expected) in cases {
            let error = token_error(guardrail.get_obo_token(assertion).await);
            assert_eq!(malformed_variant(&error), expected);
        }
        assert!(cached_tokens(&guardrail).is_empty());
    })
}

fn test_exchange_errors_carry_the_gateway_verdict() -> ScenarioFuture {
    Box::pin(async {
        let poster = Arc::new(FakePoster::default());
        poster.queue(vec![
            reply(
                401,
                r#"{"error":"invalid_client","error_description":"AADSTS700016: application not found","error_codes":[700016]}"#,
            ),
            reply(
                400,
                r#"{"error":"invalid_client","error_description":"AADSTS50027: invalid jwt token","error_codes":[50027]}"#,
            ),
        ]);
        let (guardrail, _clock) = guardrail(Arc::clone(&poster));

        let client = exchange_failure(guardrail.get_obo_token(ASSERTION).await);
        assert_eq!(client.status_code, 401);
        assert_eq!(client.error_code, "invalid_client");
        assert_eq!(client.description, "AADSTS700016: application not found");
        assert_eq!(client.aadsts_codes, vec![700_016]);
        assert!(client.gateway_owned());
        assert_eq!(
            client.to_string(),
            "invalid_client: AADSTS700016: application not found"
        );
        let assertion = exchange_failure(guardrail.get_obo_token("assertion.50027.value").await);
        assert!(!assertion.gateway_owned());
        assert_eq!(assertion.aadsts_codes, vec![50027]);
        assert!(cached_tokens(&guardrail).is_empty());
    })
}

fn test_exchange_errors_fall_back_to_defaults() -> ScenarioFuture {
    Box::pin(async {
        let poster = Arc::new(FakePoster::default());
        poster.queue(vec![
            reply(400, r#"{"error_description":"no code"}"#),
            reply(400, r#"{"error":"invalid_scope"}"#),
            reply(
                400,
                r#"{"error":null,"error_description":null,"error_codes":null}"#,
            ),
        ]);
        let (guardrail, _clock) = guardrail(Arc::clone(&poster));

        let no_code = exchange_failure(guardrail.get_obo_token(ASSERTION).await);
        assert_eq!(no_code.error_code, "invalid_grant");
        assert_eq!(no_code.description, "no code");
        assert!(no_code.aadsts_codes.is_empty());
        assert!(!no_code.gateway_owned());
        let scope = exchange_failure(guardrail.get_obo_token("second.assertion.value").await);
        assert_eq!(scope.description, "");
        assert!(scope.gateway_owned());
        let nulls = exchange_failure(guardrail.get_obo_token("third.assertion.value").await);
        assert_eq!(nulls.error_code, "invalid_grant");
        assert_eq!(nulls.description, "");
    })
}

fn test_aadsts_codes_tolerate_invalid_input() -> ScenarioFuture {
    Box::pin(async {
        let poster = Arc::new(FakePoster::default());
        poster.queue(vec![
            reply(400, r#"{"error":"invalid_client","error_codes":["50027"]}"#),
            reply(400, r#"{"error":"invalid_client","error_codes":"50027"}"#),
            reply(
                400,
                r#"{"error":"invalid_client","error_codes":[50027,"x"]}"#,
            ),
            reply(400, r#"{"error":"invalid_client","error_codes":[[50027]]}"#),
            reply(
                400,
                r#"{"error":"invalid_client","error_codes":[50027,70000]}"#,
            ),
            reply(400, r#"{"error":"invalid_client","error_codes":[]}"#),
        ]);
        let (guardrail, _clock) = guardrail(Arc::clone(&poster));
        let cases = [
            ("first.assertion.value", Vec::new()),
            ("second.assertion.value", Vec::new()),
            ("third.assertion.value", Vec::new()),
            ("fourth.assertion.value", Vec::new()),
            ("fifth.assertion.value", vec![50027, 70000]),
            ("sixth.assertion.value", Vec::new()),
        ];
        for (assertion, codes) in cases {
            let failure = exchange_failure(guardrail.get_obo_token(assertion).await);
            assert_eq!(failure.aadsts_codes, codes);
        }
    })
}

fn test_long_description_is_truncated() -> ScenarioFuture {
    Box::pin(async {
        let poster = Arc::new(FakePoster::default());
        let description = "d".repeat(600);
        let body = format!(r#"{{"error":"invalid_grant","error_description":"{description}"}}"#);
        poster.queue(vec![reply(400, &body)]);
        let (guardrail, _clock) = guardrail(Arc::clone(&poster));

        let failure = exchange_failure(guardrail.get_obo_token(ASSERTION).await);
        assert_eq!(failure.description.chars().count(), MAX_DESCRIPTION_CHARS);
    })
}

fn test_eviction_removes_only_the_named_token() -> ScenarioFuture {
    Box::pin(async {
        let poster = Arc::new(FakePoster::default());
        poster.queue(vec![
            reply(200, r#"{"access_token":"tok-1","expires_in":3600}"#),
            reply(200, r#"{"access_token":"tok-2","expires_in":3600}"#),
        ]);
        let (guardrail, _clock) = guardrail(Arc::clone(&poster));

        let _first = fetch(&guardrail, ASSERTION).await;
        guardrail.evict_obo_token(ASSERTION);
        assert!(cached_tokens(&guardrail).is_empty());
        let second = fetch(&guardrail, ASSERTION).await;

        assert_eq!(second, "tok-2");
        assert_eq!(poster.request_count(), 2);
        guardrail.evict_obo_token("never.cached.value");
        assert_eq!(cached_tokens(&guardrail).len(), 1);
    })
}

fn test_capacity_evicts_the_least_recently_used_token() -> ScenarioFuture {
    Box::pin(async {
        let poster = Arc::new(FakePoster::default());
        queue_series(&poster, OBO_CACHE_MAX_ENTRIES + 1);
        let (guardrail, _clock) = guardrail(Arc::clone(&poster));

        fetch_many(&guardrail, OBO_CACHE_MAX_ENTRIES).await;
        let promoted = fetch(&guardrail, "assertion-0").await;
        let newest = fetch(&guardrail, "assertion-1000").await;

        assert_eq!(promoted, "tok-0");
        assert_eq!(newest, "tok-1000");
        assert_eq!(poster.request_count(), OBO_CACHE_MAX_ENTRIES + 1);
        let tokens = cached_tokens(&guardrail);
        assert_eq!(tokens.len(), OBO_CACHE_MAX_ENTRIES);
        assert_eq!(tokens.first().unwrap().1, "tok-2");
        assert_eq!(tokens.last().unwrap().1, "tok-1000");
        let keys = cached_keys(&guardrail);
        assert!(keys.contains(&cache_key("assertion-0")));
        assert!(!keys.contains(&cache_key("assertion-1")));
    })
}

fn test_assertion_shapes() -> ScenarioFuture {
    Box::pin(async {
        let compact = GuardrailValue::Text("a.b.c".to_owned());
        assert_eq!(entra_assertion(&compact), Some("a.b.c"));
        for rejected in [
            GuardrailValue::Text("sk-virtual-key".to_owned()),
            GuardrailValue::Text("a.b".to_owned()),
            GuardrailValue::Text("a.b.c.d".to_owned()),
            GuardrailValue::Text(String::new()),
            GuardrailValue::Null,
            GuardrailValue::SignedInteger(3),
        ] {
            assert_eq!(entra_assertion(&rejected), None);
        }
    })
}

fn test_post_returns_the_reply() -> ScenarioFuture {
    Box::pin(async {
        let poster = Arc::new(FakePoster::default());
        poster.queue(vec![
            reply(503, r#"{"error":"unavailable"}"#),
            reply(404, ""),
        ]);
        let (guardrail, _clock) = guardrail(Arc::clone(&poster));
        let headers =
            HeaderMap::from_iter([(CONTENT_TYPE, HeaderValue::from_static("application/json"))]);

        let first = guardrail
            .post_allowing_error_status("https://agent.example/evaluate", &headers, b"{}")
            .await
            .unwrap();
        let second = guardrail
            .post_allowing_error_status("https://agent.example/evaluate", &headers, b"")
            .await
            .unwrap();

        assert_eq!(first.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(first.body, br#"{"error":"unavailable"}"#.to_vec());
        assert_eq!(second.status, StatusCode::NOT_FOUND);
        assert!(second.body.is_empty());
        let recorded = poster.recorded();
        assert_eq!(recorded.len(), 2);
        let request = recorded.first().unwrap();
        assert_eq!(request.url, "https://agent.example/evaluate");
        assert_eq!(request.content_type.as_deref(), Some("application/json"));
        assert_eq!(request.timeout, Duration::from_secs(10));
    })
}

fn test_http_poster_returns_error_statuses() -> ScenarioFuture {
    Box::pin(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        // 4 KiB: past a 1 KiB limit, well inside the 1 MiB one
        let body = "x".repeat(4096);
        let response = format!(
            "HTTP/1.1 503 Service Unavailable\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        );
        let mut tasks = JoinSet::new();
        tasks.spawn(async move {
            let (mut stream, _peer) = listener.accept().await?;
            let mut buffer = vec![0_u8; 4096];
            let _read = stream.read(&mut buffer).await?;
            stream.write_all(response.as_bytes()).await
        });
        let poster = HttpPoster {
            client: base_builder().build().unwrap(),
        };
        let reply = poster
            .post(
                &format!("http://{address}/token"),
                &HeaderMap::new(),
                b"grant_type=x",
                Duration::from_secs(5),
            )
            .await
            .unwrap();
        assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(reply.body, body.as_bytes());
        let joined = tasks.join_all().await;
        assert!(joined.iter().all(Result::is_ok));
    })
}

fn test_constants() -> ScenarioFuture {
    Box::pin(async {
        assert_eq!(
            AGENT_365_PROD_API_BASE,
            "https://agent365.svc.cloud.microsoft"
        );
        assert_eq!(
            AGENT_365_PROD_RESOURCE_APP_ID,
            "ea9ffc3e-8a23-4a7d-836d-234d7c7565c1"
        );
        assert_eq!(AGENT_365_SCOPE_NAME, "ThreatProtection.Evaluate.All");
        assert_eq!(
            EVALUATE_URL,
            format!("{AGENT_365_PROD_API_BASE}/agents/tool-evaluation/evaluate")
        );
        assert_eq!(
            OBO_SCOPE,
            format!("{AGENT_365_PROD_RESOURCE_APP_ID}/{AGENT_365_SCOPE_NAME}")
        );
        assert_eq!(DEFENDER_STATUS_EVALUATED, "Evaluated");
        assert_eq!(SUPPORTED_EVENT_HOOKS, [GuardrailEventHooks::PreMcpCall]);
        let default_timeout = Duration::try_from_secs_f64(DEFAULT_TIMEOUT_SECONDS).unwrap();
        assert_eq!(default_timeout, Duration::from_secs(10));
        assert_eq!(DEFAULT_TOKEN_TTL, Duration::from_secs(3599));
        assert_eq!(TOKEN_EXPIRY_MARGIN, Duration::from_secs(60));
        assert_eq!(OBO_CACHE_MAX_ENTRIES, 1000);
    })
}

fn test_wiring() -> ScenarioFuture {
    Box::pin(async {
        let poster: Arc<dyn Agent365Poster> = Arc::new(FakePoster::default());
        let clock = TestClock::new();
        let guardrail =
            Agent365Guardrail::new(settings(), Arc::clone(&poster), true, clock.handle()).unwrap();
        assert_eq!(guardrail.request_timeout, Duration::from_secs(10));
        assert_eq!((guardrail.clock)(), start_time());
        assert_eq!(
            guardrail.base.settings.supported_event_hooks,
            Some(vec![GuardrailEventHooks::PreMcpCall])
        );
        let name = guardrail.base.settings.guardrail_name.as_deref();
        assert_eq!(name, Some("agent365"));
        assert!(guardrail.base.settings.run_in_parallel);
        assert!(guardrail.base.settings.switches.records_own_information);
        assert!(!guardrail.base.settings.switches.use_native_lifecycle_hooks);
        let default_fallback = guardrail.settings.unreachable_fallback;
        assert_eq!(default_fallback, UnreachableFallback::FailClosed);

        let mut explicit = settings();
        explicit.request_timeout = Some(2.5_f64);
        explicit.event_hook = Some(LitellmParamsMode::One("pre_mcp_call".to_owned()));
        explicit.default_on = true;
        explicit.unreachable_fallback = UnreachableFallback::FailOpen;
        let configured = Agent365Guardrail::new(explicit, poster, true, clock.handle()).unwrap();
        assert_eq!(configured.request_timeout, Duration::from_millis(2500));
        assert_eq!(
            configured.base.settings.event_hook,
            Some(LitellmParamsMode::One("pre_mcp_call".to_owned()))
        );
        assert!(configured.base.settings.default_on);
        let configured_fallback = configured.settings.unreachable_fallback;
        assert_eq!(configured_fallback, UnreachableFallback::FailOpen);
    })
}

fn test_invalid_timeouts_are_rejected() -> ScenarioFuture {
    Box::pin(async {
        for seconds in [-1.0_f64, f64::NAN, f64::INFINITY] {
            let mut invalid = settings();
            invalid.request_timeout = Some(seconds);
            let poster = Arc::new(FakePoster::default());
            let result = Agent365Guardrail::new(invalid, poster, true, TestClock::new().handle());
            match result {
                Err(Agent365Error::Timeout { source, .. }) => {
                    assert!(!source.to_string().is_empty());
                }
                Err(other) => panic!("expected a timeout error, got {other:?}"),
                Ok(_) => panic!("expected a timeout error"),
            }
        }
    })
}

#[rstest]
#[case::token_is_cached_per_assertion(test_token_is_cached_per_assertion)]
#[case::token_within_the_margin_is_reused(test_token_within_the_margin_is_reused)]
#[case::token_at_the_margin_is_refetched(test_token_at_the_margin_is_refetched)]
#[case::expired_token_is_refetched(test_expired_token_is_refetched)]
#[case::expires_in_forms_are_parsed(test_expires_in_forms_are_parsed)]
#[case::default_ttl_applies_until_the_margin(test_default_ttl_applies_until_the_margin)]
#[case::throttled_and_server_errors(test_throttled_and_server_errors)]
#[case::malformed_bodies(test_malformed_bodies)]
#[case::exchange_errors_carry_the_gateway_verdict(test_exchange_errors_carry_the_gateway_verdict)]
#[case::exchange_errors_fall_back_to_defaults(test_exchange_errors_fall_back_to_defaults)]
#[case::aadsts_codes_tolerate_invalid_input(test_aadsts_codes_tolerate_invalid_input)]
#[case::long_description_is_truncated(test_long_description_is_truncated)]
#[case::eviction_removes_only_the_named_token(test_eviction_removes_only_the_named_token)]
#[case::capacity_evicts_the_least_recently_used_token(
    test_capacity_evicts_the_least_recently_used_token
)]
#[case::assertion_shapes(test_assertion_shapes)]
#[case::post_returns_the_reply(test_post_returns_the_reply)]
#[case::http_poster_returns_error_statuses(test_http_poster_returns_error_statuses)]
#[case::constants(test_constants)]
#[case::wiring(test_wiring)]
#[case::invalid_timeouts_are_rejected(test_invalid_timeouts_are_rejected)]
#[case::lifetime_past_the_clock_range_is_refused(test_lifetime_past_the_clock_range_is_refused)]
#[tokio::test]
async fn agent_365_token_behaviors(#[case] scenario: Scenario) {
    scenario().await;
}
