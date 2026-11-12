use rstest::rstest;
use serde::Deserialize;

use super::{ANSWER, OWN_KEY, PLAIN, call};

#[derive(Deserialize)]
struct Reply {
    error: Detail,
}

#[derive(Deserialize)]
struct Detail {
    #[serde(rename = "type")]
    error_type: String,
    code: String,
}

const THROTTLED: &str = r#"{"error":{"type":"requests","param":"p"}}"#;
const REFUSED: &str = r#"{"error":{"message":"bad key"}}"#;

#[rstest]
#[case::aiohttp_without_a_key_sends_bearer_none(
    "aiohttp_openai/up",
    "",
    (200, ANSWER),
    None,
    "authorization: bearer none"
)]
#[case::custom_llm_provider_selects_aiohttp(
    "up, custom_llm_provider: aiohttp_openai",
    OWN_KEY,
    (200, ANSWER),
    None,
    "authorization: bearer own-key"
)]
#[case::aiohttp_keeps_a_rate_limit_status(
    "aiohttp_openai/up",
    OWN_KEY,
    (429, THROTTLED),
    Some("throttling_error"),
    "authorization: bearer own-key"
)]
#[case::aiohttp_keeps_an_authentication_status(
    "aiohttp_openai/up",
    OWN_KEY,
    (401, REFUSED),
    Some("authentication_error"),
    "authorization: bearer own-key"
)]
#[tokio::test]
async fn provider_framework_tests_behavior(
    #[case] model: &str,
    #[case] key: &str,
    #[case] upstream_reply: (u16, &str),
    #[case] error_type: Option<&str>,
    #[case] authorization: &str,
) {
    let (answered, body, sent, _forwarded) = call(model, PLAIN, key, upstream_reply).await;
    assert_eq!(answered, upstream_reply.0);
    assert!(sent.contains(authorization));
    assert!(!sent.contains("env-key"));
    assert!(!sent.contains("openai-organization"));
    if let Some(expected) = error_type {
        let reply: Reply = serde_json::from_slice(&body).unwrap();
        assert_eq!(reply.error.error_type, expected);
        assert_eq!(reply.error.code, upstream_reply.0.to_string());
    }
}
