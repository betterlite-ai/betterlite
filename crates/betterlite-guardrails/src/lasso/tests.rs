#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt;
use std::future::IntoFuture as _;
use std::sync::Mutex;
use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use axum::routing::post;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::lasso_classification::Configured;
use reqwest::header::HeaderMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::value::RawValue;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use ulid::Ulid;

use super::error::LassoError;
use super::wire::{Finding, LassoRequestData, LassoResponse, MessageType, ToolCall};
use super::{ConversationCache, DEFAULT_TIMEOUT, LassoGuardrail};

#[derive(Default)]
struct TestCache {
    entries: Mutex<BTreeMap<String, (String, Duration)>>,
    broken: bool,
}

impl ConversationCache for TestCache {
    type Error = fmt::Error;

    fn get_cache(&self, key: &str) -> Result<Option<String>, fmt::Error> {
        if self.broken {
            return Err(fmt::Error);
        }
        let entries = self.entries.lock().map_err(|_poisoned| fmt::Error)?;
        Ok(entries.get(key).map(|(value, _)| value.clone()))
    }

    fn set_cache(&self, key: &str, value: &str, ttl: Duration) -> Result<(), fmt::Error> {
        if self.broken {
            return Err(fmt::Error);
        }
        let entry = (value.to_owned(), ttl);
        let mut entries = self.entries.lock().map_err(|_poisoned| fmt::Error)?;
        entries.insert(key.to_owned(), entry);
        drop(entries);
        Ok(())
    }
}

fn fixed_id() -> Ulid {
    Ulid::from_parts(1, 2)
}

fn settings(user_id: Option<&str>, conversation_id: Option<&str>) -> Configured {
    Configured {
        guardrail_name: "lasso".to_owned(),
        api_key: SecretString::from("key"),
        api_base: "http://127.0.0.1:9".to_owned(),
        user_id: user_id.map(ToOwned::to_owned),
        conversation_id: conversation_id.map(ToOwned::to_owned),
        mask: false,
    }
}

fn guardrail(settings: &Configured, timeout: Duration) -> LassoGuardrail {
    LassoGuardrail::new(settings, reqwest::Client::default(), timeout, fixed_id)
}

#[test]
fn lasso_guardrail_reads_config_and_environment() {
    let lookup = BTreeMap::from([
        ("LASSO_API_KEY".to_owned(), "env-key".to_owned()),
        ("LASSO_USER_ID".to_owned(), "env-user".to_owned()),
    ]);
    let yaml =
        "guardrails:\n  - guardrail_name: lasso\n    litellm_params:\n      guardrail: lasso\n";
    let config = ConfigLoader { lookup }.load(yaml).unwrap();
    let lasso = guardrail(
        &config.sections.lasso_classification.guardrails[0],
        DEFAULT_TIMEOUT,
    );
    let headers = lasso.prepare_headers(None, &TestCache::default()).unwrap();
    assert_eq!(headers.get("lasso-api-key").unwrap(), "env-key");
    assert_eq!(headers.get("lasso-user-id").unwrap(), "env-user");
}

#[test]
fn lasso_guardrail_stores_generated_conversation_id_for_an_hour() {
    let cache = TestCache::default();
    let lasso = guardrail(&settings(None, None), DEFAULT_TIMEOUT);
    let id = lasso.conversation_id(Some("call-1"), &cache);
    assert_eq!(id, fixed_id().to_string());
    let stored = cache.entries.into_inner().unwrap();
    let entry = stored.get("lasso_conversation_id:call-1");
    assert_eq!(entry, Some(&(id, Duration::from_secs(3_600))));
}

#[rstest]
#[case::configured_wins(Some("session"), Some("call-1"), false, "session")]
#[case::cache_hit(None, Some("call-1"), false, "cached")]
#[case::no_call_id(None, None, false, "00000000010000000000000002")]
#[case::broken_cache(None, Some("call-1"), true, "00000000010000000000000002")]
fn lasso_guardrail_conversation_id(
    #[case] configured: Option<&str>,
    #[case] call_id: Option<&str>,
    #[case] broken: bool,
    #[case] expected: &str,
) {
    let cached = ("cached".to_owned(), Duration::ZERO);
    let entries = BTreeMap::from([("lasso_conversation_id:call-1".to_owned(), cached)]);
    let cache = TestCache {
        entries: Mutex::new(entries),
        broken,
    };
    let lasso = guardrail(&settings(None, configured), DEFAULT_TIMEOUT);
    assert_eq!(lasso.conversation_id(call_id, &cache), expected);
}

#[rstest]
#[case::object(
    r#"{"id":"c1","function":{"name":"f","arguments":"{\"a\":1}"}}"#,
    r#"{"id":"c1","name":"f","input":{"a":1}}"#
)]
#[case::malformed(
    r#"{"function":{"name":"f","arguments":"smuggled PII"}}"#,
    r#"{"id":null,"name":"f","input":{"arguments":"smuggled PII"}}"#
)]
#[case::not_an_object(
    r#"{"function":{"arguments":"[1,2]"}}"#,
    r#"{"id":null,"name":null,"input":{"arguments":"[1,2]"}}"#
)]
#[case::empty(
    r#"{"function":{"name":"f","arguments":""}}"#,
    r#"{"id":null,"name":"f","input":null}"#
)]
#[case::no_function(r#"{"id":"c1"}"#, r#"{"id":"c1","name":null,"input":null}"#)]
fn lasso_guardrail_tool_call_fields(#[case] json: &str, #[case] expected: &str) {
    let call: ToolCall = serde_json::from_str(json).unwrap();
    let fields = super::extract_tool_call_fields(&call);
    assert_eq!(serde_json::to_string(&fields).unwrap(), expected);
}

#[rstest]
#[case::with_user(Some("user"), Some("user"))]
#[case::without_user(None, None)]
fn lasso_guardrail_headers(#[case] user_id: Option<&str>, #[case] expected_user: Option<&str>) {
    let lasso = guardrail(&settings(user_id, None), DEFAULT_TIMEOUT);
    let headers = lasso.prepare_headers(None, &TestCache::default()).unwrap();
    let header = |name: &str| headers.get(name).map(|value| value.to_str().unwrap());
    let conversation_id = fixed_id().to_string();
    assert_eq!(header("lasso-api-key"), Some("key"));
    assert_eq!(header("content-type"), Some("application/json"));
    assert_eq!(header("lasso-user-id"), expected_user);
    assert_eq!(
        header("lasso-conversation-id"),
        Some(conversation_id.as_str())
    );
}

#[test]
fn lasso_guardrail_payload_flattens_tools_and_tags_the_source() {
    let data: LassoRequestData = serde_json::from_str(
        r#"{"litellm_call_id":"call-1","tools":[{"function":{"name":"f","description":"","parameters":{}}},{"function":{"name":"","description":"skip"}},{"type":"function"},{"function":{"name":"g","description":"d","parameters":{"type":"object"}}}]}"#,
    )
    .unwrap();
    let message = RawValue::from_string(r#"{"role":"user","content":"hello"}"#.to_owned()).unwrap();
    let lasso = guardrail(&settings(Some("user"), Some("session")), DEFAULT_TIMEOUT);
    let cache = TestCache::default();
    let payload = lasso.prepare_payload(vec![message], &data, MessageType::Completion, &cache);
    assert_eq!(
        serde_json::to_string(&payload).unwrap(),
        r#"{"messages":[{"role":"user","content":"hello"}],"messageType":"COMPLETION","sessionId":"session","userId":"user","tools":[{"name":"f"},{"name":"g","description":"d","parameters":{"type":"object"}}],"source":{"type":"litellm"}}"#
    );
}

#[rstest]
#[case::block(true, Some("BLOCK"), true)]
#[case::auto_masking(true, Some("AUTO_MASKING"), false)]
#[case::warn(true, Some("WARN"), false)]
#[case::missing_action(true, None, false)]
#[case::no_violation(false, Some("BLOCK"), false)]
fn lasso_guardrail_blocks_only_block_actions(
    #[case] violations_detected: bool,
    #[case] action: Option<&str>,
    #[case] blocked: bool,
) {
    let finding = Finding {
        action: action.map(ToOwned::to_owned),
    };
    let response = LassoResponse {
        violations_detected,
        deputies: BTreeMap::from([("jailbreak".to_owned(), true)]),
        findings: BTreeMap::from([("jailbreak".to_owned(), vec![finding])]),
    };
    let result = guardrail(&settings(None, None), DEFAULT_TIMEOUT).process_response(response);
    assert_eq!(matches!(result, Err(LassoError::Blocked { .. })), blocked);
}

async fn call_mock(
    route: Option<(&str, StatusCode)>,
    api_url: Option<&str>,
) -> Result<LassoResponse, LassoError> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut configured = settings(None, None);
    configured.api_base.clone_from(&base);
    let lasso = guardrail(&configured, Duration::from_millis(200));
    let data = serde_json::from_str("{}").unwrap();
    let payload = lasso.prepare_payload(
        Vec::new(),
        &data,
        MessageType::Prompt,
        &TestCache::default(),
    );
    let mut server = JoinSet::new();
    if let Some((path, status)) = route {
        let answer = move || async move { (status, r#"{"violations_detected":false}"#) };
        let router = Router::new().route(path, post(answer));
        server.spawn(axum::serve(listener, router).into_future());
    }
    let url = api_url.map(|path| format!("{base}{path}"));
    lasso
        .call_api(HeaderMap::new(), &payload, url.as_deref())
        .await
}

#[rstest]
#[case::classify_by_default(Some(("/classify", StatusCode::OK)), None, "ok")]
#[case::explicit_classifix(Some(("/classifix", StatusCode::OK)), Some("/classifix"), "ok")]
#[case::invalid_key(
    Some(("/classify", StatusCode::UNAUTHORIZED)),
    None,
    "the Lasso API key was rejected"
)]
#[case::rate_limit(
    Some(("/classify", StatusCode::TOO_MANY_REQUESTS)),
    None,
    "the Lasso API rate limit is exceeded"
)]
#[case::server_error(
    Some(("/classify", StatusCode::BAD_GATEWAY)),
    None,
    "the Lasso API answered with status 502 Bad Gateway"
)]
#[case::timeout(None, None, "the Lasso API did not answer in time")]
#[tokio::test]
async fn lasso_guardrail_call_api(
    #[case] route: Option<(&str, StatusCode)>,
    #[case] api_url: Option<&str>,
    #[case] expected: &str,
) {
    let outcome = call_mock(route, api_url).await;
    let text = outcome.map_or_else(|error| error.to_string(), |_| "ok".to_owned());
    assert_eq!(text, expected);
}
