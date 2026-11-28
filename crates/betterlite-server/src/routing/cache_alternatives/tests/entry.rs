use std::io;
use std::str;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::cache_cost_routing as section;
use betterlite_models::decisions::Json;
use betterlite_prompts::message_content::Message;
use betterlite_router::cache_cost_routing::{CacheAwareChoice, ChooseCached, PreRoutingResponse};
use betterlite_router::router_predicates::JsonMap;
use rstest::rstest;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use crate::routing::cache_alternatives::entry::choose_cached_model;
use crate::routing::cache_alternatives::tests::support::{
    CALLER, FakeHost, FakeLimiter, FakeTierParams, Fixture, Refusal, STRONG_ID, STRONG_MODEL,
    UNAVAILABLE_LOG, body, decimal, json, marker_deployment, messages, request_kwargs, response,
    seed, tier_limits,
};

const CONTEXT: &str = "You are a helpful assistant.";

fn entry_case(request_body: &Json) -> EntryCase {
    EntryCase {
        config: section::Config {
            enabled: true,
            output_tokens: 1024,
            timeout_ms: 2000,
        },
        response: response("SIMPLE", "cheap"),
        request_kwargs: request_kwargs(CALLER, request_body),
        messages: messages(request_body),
        params: FakeTierParams::default(),
    }
}

// choose_cached_model's arguments for one case.
struct EntryCase {
    config: section::Config,
    response: PreRoutingResponse,
    request_kwargs: JsonMap,
    messages: Vec<Message>,
    params: FakeTierParams,
}

impl EntryCase {
    async fn run(&self, host: &FakeHost<'_>) -> Option<CacheAwareChoice> {
        choose_cached_model(
            host,
            &ChooseCached {
                config: self.config,
                params_for_model: &self.params,
                response: &self.response,
                request_kwargs: &self.request_kwargs,
                messages: Some(self.messages.as_slice()),
            },
        )
        .await
    }
}

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        str::from_utf8(&bytes)
            .expect("the captured log lines are UTF-8")
            .to_owned()
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

fn capture_logs(level: Level) -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(level)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}

#[tokio::test]
async fn a_warm_stronger_model_wins_through_the_entry_point() {
    let fixture = Fixture::new();
    let request_body = body(CONTEXT);
    let case = entry_case(&request_body);
    seed(&fixture.cache, &request_body, 1290.0);
    let context = fixture.context();
    let host = fixture.host(&context);
    let choice = case.run(&host).await.expect("the warm alternative wins");
    assert_eq!(choice.model, STRONG_MODEL);
    assert_eq!(choice.deployment_id, STRONG_ID);
    assert_eq!(choice.original_cost, decimal("0.067596"));
    assert_eq!(choice.estimated_cost, decimal("0.03336"));
    assert_eq!(host.requests(), 4);
    assert_eq!(
        host.authorization_calls(),
        vec!["cheap".to_owned(), "strong".to_owned()]
    );
}

#[tokio::test]
async fn a_disabled_setting_touches_nothing() {
    let fixture = Fixture::new();
    let request_body = body(CONTEXT);
    let mut case = entry_case(&request_body);
    case.config.enabled = false;
    seed(&fixture.cache, &request_body, 1290.0);
    let context = fixture.context();
    let host = fixture.host(&context);
    let (logs, _guard) = capture_logs(Level::DEBUG);
    assert!(case.run(&host).await.is_none());
    assert_eq!(host.requests(), 0);
    assert!(host.authorization_calls().is_empty());
    assert!(!logs.contents().contains(UNAVAILABLE_LOG));
}

#[rstest]
#[case::foreign_router(Refusal::ForeignRouter)]
#[case::routing_plugins(Refusal::RoutingPlugins)]
#[case::request_transforms(Refusal::RequestTransforms)]
#[case::compaction_pending(Refusal::CompactionPending)]
#[tokio::test]
async fn each_proxy_state_gate_keeps_the_classified_model(#[case] refusal: Refusal) {
    let fixture = Fixture::new();
    let request_body = body(CONTEXT);
    let case = entry_case(&request_body);
    seed(&fixture.cache, &request_body, 1290.0);
    let context = fixture.context();
    let (logs, _guard) = capture_logs(Level::DEBUG);
    let mut host = fixture.host(&context);
    host.refusals.push(refusal);
    assert!(case.run(&host).await.is_none(), "refusal {refusal:?}");
    assert_eq!(host.requests(), 0);
    assert!(host.authorization_calls().is_empty());
    assert!(!logs.contents().contains(UNAVAILABLE_LOG));
}

#[tokio::test]
async fn an_unsupported_router_marker_keeps_the_classified_model() {
    let fixture = Fixture::new();
    let request_body = body(CONTEXT);
    let case = entry_case(&request_body);
    seed(&fixture.cache, &request_body, 1290.0);
    let context = fixture.context();
    let mut host = fixture.host(&context);
    let extra = r#"{"model":"auto_router/complexity_router","temperature":0.1}"#;
    host.deployments
        .insert("smart".to_owned(), vec![marker_deployment(extra)]);
    assert!(case.run(&host).await.is_none());
    assert_eq!(host.requests(), 0);
    assert!(host.authorization_calls().is_empty());
}

#[tokio::test]
async fn a_missing_caller_keeps_the_classified_model() {
    let fixture = Fixture::new();
    let request_body = body(CONTEXT);
    let mut case = entry_case(&request_body);
    assert!(case.request_kwargs.remove("litellm_metadata").is_some());
    seed(&fixture.cache, &request_body, 1290.0);
    let context = fixture.context();
    let host = fixture.host(&context);
    assert!(case.run(&host).await.is_none());
    assert_eq!(host.requests(), 0);
}

#[tokio::test]
async fn a_non_empty_per_key_config_is_refused() {
    let fixture = Fixture::new();
    let request_body = body(CONTEXT);
    let mut case = entry_case(&request_body);
    let Some(Json::Object(metadata)) = case.request_kwargs.get_mut("litellm_metadata") else {
        panic!("the metadata bucket is an object");
    };
    let Some(Json::Object(auth)) = metadata.get_mut("user_api_key_auth") else {
        panic!("the caller entry is an object");
    };
    auth.insert(
        "config".to_owned(),
        json(r#"{"routing":{"enabled":false}}"#),
    );
    seed(&fixture.cache, &request_body, 1290.0);
    let context = fixture.context();
    let host = fixture.host(&context);
    assert!(case.run(&host).await.is_none());
    assert_eq!(host.requests(), 0);
}

#[tokio::test]
async fn a_per_key_config_of_another_type_logs_once() {
    let fixture = Fixture::new();
    let request_body = body(CONTEXT);
    let mut case = entry_case(&request_body);
    let Some(Json::Object(metadata)) = case.request_kwargs.get_mut("litellm_metadata") else {
        panic!("the metadata bucket is an object");
    };
    let Some(Json::Object(auth)) = metadata.get_mut("user_api_key_auth") else {
        panic!("the caller entry is an object");
    };
    auth.insert("config".to_owned(), Json::Text("deny".to_owned()));
    seed(&fixture.cache, &request_body, 1290.0);
    let context = fixture.context();
    let host = fixture.host(&context);
    let (logs, _guard) = capture_logs(Level::DEBUG);
    assert!(case.run(&host).await.is_none());
    assert_eq!(host.requests(), 0);
    assert_eq!(logs.contents().matches(UNAVAILABLE_LOG).count(), 1);
}

#[tokio::test]
async fn a_bad_proxy_request_keeps_the_classified_model() {
    let fixture = Fixture::new();
    let request_body = body(CONTEXT);
    let mut case = entry_case(&request_body);
    assert!(case.request_kwargs.remove("proxy_server_request").is_some());
    seed(&fixture.cache, &request_body, 1290.0);
    let context = fixture.context();
    let host = fixture.host(&context);
    assert!(case.run(&host).await.is_none());
    assert_eq!(host.requests(), 0);
}

#[tokio::test]
async fn a_refused_request_field_keeps_the_classified_model() {
    let fixture = Fixture::new();
    let request_body = body(CONTEXT);
    let mut case = entry_case(&request_body);
    case.request_kwargs
        .insert("api_base".to_owned(), json(r#""https://example.test""#));
    seed(&fixture.cache, &request_body, 1290.0);
    let context = fixture.context();
    let host = fixture.host(&context);
    assert!(case.run(&host).await.is_none());
    assert_eq!(host.requests(), 0);
}

#[tokio::test]
async fn an_empty_refused_field_still_compares() {
    let fixture = Fixture::new();
    let request_body = body(CONTEXT);
    let mut case = entry_case(&request_body);
    case.request_kwargs
        .insert("guardrails".to_owned(), Json::List(Vec::new()));
    seed(&fixture.cache, &request_body, 1290.0);
    let context = fixture.context();
    let host = fixture.host(&context);
    let choice = case.run(&host).await.expect("an empty list passes");
    assert_eq!(choice.model, STRONG_MODEL);
}

#[tokio::test]
async fn a_missing_limiter_keeps_the_classified_model() {
    let fixture = Fixture::new();
    let request_body = body(CONTEXT);
    let case = entry_case(&request_body);
    seed(&fixture.cache, &request_body, 1290.0);
    let context = fixture.context();
    let mut host = fixture.host(&context);
    host.limiter = None;
    assert!(case.run(&host).await.is_none());
    assert_eq!(host.requests(), 0);
}

#[tokio::test]
async fn a_denied_capacity_yields_no_estimate() {
    let fixture = Fixture::new();
    let request_body = body(CONTEXT);
    let case = entry_case(&request_body);
    seed(&fixture.cache, &request_body, 1290.0);
    let context = fixture.context();
    let mut host = fixture.host(&context);
    host.limiter = Some(FakeLimiter::denying());
    assert!(case.run(&host).await.is_none());
    assert_eq!(host.requests(), 0);
    let limiter = host
        .limiter
        .as_ref()
        .expect("the limiter stays on the host");
    assert_eq!(limiter.grants(), vec![STRONG_MODEL.to_owned()]);
}

#[tokio::test(start_paused = true)]
async fn the_deadline_keeps_the_classified_model_and_logs_once() {
    let fixture = Fixture::new();
    let request_body = body(CONTEXT);
    let mut case = entry_case(&request_body);
    case.config.timeout_ms = 1;
    seed(&fixture.cache, &request_body, 1290.0);
    let context = fixture.context();
    let mut host = fixture.host(&context);
    host.limiter = Some(FakeLimiter::hanging());
    let (logs, _guard) = capture_logs(Level::DEBUG);
    assert!(case.run(&host).await.is_none());
    assert_eq!(host.requests(), 0);
    assert_eq!(logs.contents().matches(UNAVAILABLE_LOG).count(), 1);
}

#[tokio::test]
async fn a_host_error_keeps_the_classified_model_and_logs_once() {
    let fixture = Fixture::new();
    let request_body = body(CONTEXT);
    let case = entry_case(&request_body);
    seed(&fixture.cache, &request_body, 1290.0);
    let context = fixture.context();
    let mut host = fixture.host(&context);
    host.missing.push("strong".to_owned());
    let (logs, _guard) = capture_logs(Level::DEBUG);
    assert!(case.run(&host).await.is_none());
    assert_eq!(logs.contents().matches(UNAVAILABLE_LOG).count(), 1);
}

#[tokio::test]
async fn an_ordinary_none_logs_nothing() {
    let fixture = Fixture::new();
    let request_body = body(CONTEXT);
    let case = entry_case(&request_body);
    let context = fixture.context();
    let host = fixture.host(&context);
    let (logs, _guard) = capture_logs(Level::DEBUG);
    assert!(case.run(&host).await.is_none());
    assert_eq!(host.requests(), 0);
    assert!(!logs.contents().contains(UNAVAILABLE_LOG));
}

#[tokio::test]
async fn an_unsupported_messages_url_keeps_the_classified_model() {
    let fixture = Fixture::new();
    let request_body = body(CONTEXT);
    let mut case = entry_case(&request_body);
    let Some(Json::Object(proxy)) = case.request_kwargs.get_mut("proxy_server_request") else {
        panic!("the proxy request is an object");
    };
    proxy.insert(
        "url".to_owned(),
        Json::Text("https://api.anthropic.com/v1/complete".to_owned()),
    );
    seed(&fixture.cache, &request_body, 1290.0);
    let context = fixture.context();
    let host = fixture.host(&context);
    assert!(case.run(&host).await.is_none());
    assert_eq!(host.requests(), 0);
}

#[tokio::test]
async fn the_tier_limits_reach_the_entry_point() {
    let fixture = Fixture::new();
    let request_body = body(CONTEXT);
    let mut case = entry_case(&request_body);
    case.params = tier_limits(&[("SIMPLE", "cheap", 50), ("COMPLEX", "strong", 100)]);
    seed(&fixture.cache, &request_body, 1290.0);
    let context = fixture.context();
    let host = fixture.host(&context);
    let choice = case.run(&host).await.expect("the capped alternative wins");
    assert_eq!(choice.original_cost, decimal("0.0637"));
    assert_eq!(choice.estimated_cost, decimal("0.0195"));
}

#[tokio::test]
async fn a_tier_context_that_cannot_fit_keeps_the_classified_model() {
    let fixture = Fixture::new();
    let request_body = body(CONTEXT);
    let mut case = entry_case(&request_body);
    case.params = tier_limits(&[("COMPLEX", "strong", 60_000)]);
    seed(&fixture.cache, &request_body, 1290.0);
    let context = fixture.context();
    let host = fixture.host(&context);
    assert!(case.run(&host).await.is_none());
    assert_eq!(host.requests(), 4);
}

#[tokio::test]
async fn a_marker_lookup_error_logs_once() {
    let fixture = Fixture::new();
    let request_body = body(CONTEXT);
    let case = entry_case(&request_body);
    seed(&fixture.cache, &request_body, 1290.0);
    let context = fixture.context();
    let mut host = fixture.host(&context);
    host.missing.push("smart".to_owned());
    let (logs, _guard) = capture_logs(Level::DEBUG);
    assert!(case.run(&host).await.is_none());
    assert_eq!(logs.contents().matches(UNAVAILABLE_LOG).count(), 1);
}
