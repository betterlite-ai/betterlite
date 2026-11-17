use secrecy::ExposeSecret as _;

use super::support;
use crate::openai_dispatch::call::ChatCall;
use crate::openai_dispatch::compose::resolve;
use crate::openai_dispatch::fixtures::{FixtureAnswer, FixtureLogger};

fn call(headers: Option<betterlite_openai::openai_base_config::Headers>) -> ChatCall {
    ChatCall {
        model: String::from("gpt-4o"),
        request: support::chat_request(
            r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#,
        ),
        credentials: support::credentials(None, None, None),
        headers,
        extra_headers: None,
        timeout: None,
        metadata: None,
    }
}

fn tenant_headers(tenant: &str) -> betterlite_openai::openai_base_config::Headers {
    betterlite_openai::openai_base_config::Headers::from([(
        String::from("x-tenant"),
        tenant.to_owned(),
    )])
}

// Two configured identities keep their own base, key, organization and headers on every call.
#[tokio::test]
async fn openai_provider_credential_identity_isolation() {
    let mut first = support::composition();
    first.globals = support::globals(
        Some("https://first.test/v1"),
        Some("sk-first"),
        None,
        Some("org-first"),
    );
    let mut second = support::composition();
    second.globals = support::globals(
        Some("https://second.test/v1"),
        Some("sk-second"),
        None,
        Some("org-second"),
    );
    let plain = support::credentials(None, None, None);
    let resolved_first = resolve(&first, &plain);
    let resolved_second = resolve(&second, &plain);
    assert_eq!(resolved_first.api_base, "https://first.test/v1");
    assert_eq!(resolved_second.api_base, "https://second.test/v1");
    assert_eq!(
        resolved_first
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("sk-first")
    );
    assert_eq!(
        resolved_second
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("sk-second")
    );
    assert_eq!(resolved_first.organization.as_deref(), Some("org-first"));
    assert_eq!(resolved_second.organization.as_deref(), Some("org-second"));

    // an explicit identity on one call is never memoized into the composition or the next call
    let explicit = support::credentials(
        Some("https://explicit.test/v1"),
        Some("sk-explicit"),
        Some("org-explicit"),
    );
    let resolved_explicit = resolve(&first, &explicit);
    assert_eq!(resolved_explicit.api_base, "https://explicit.test/v1");
    let resolved_again = resolve(&first, &plain);
    assert_eq!(resolved_again.api_base, "https://first.test/v1");
    assert_eq!(
        resolved_again
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("sk-first")
    );

    // each call's headers reach only its own wire request
    let first_answer = FixtureAnswer::new(200, support::CHAT_REPLY.as_bytes().to_vec());
    let mut first_logger = FixtureLogger::default();
    let _result = support::dispatch_chat(
        &first,
        call(Some(tenant_headers("first"))),
        &first_answer,
        &mut first_logger,
    )
    .await
    .expect("the first chat dispatch");
    let second_answer = FixtureAnswer::new(200, support::CHAT_REPLY.as_bytes().to_vec());
    let mut second_logger = FixtureLogger::default();
    let _result = support::dispatch_chat(
        &second,
        call(Some(tenant_headers("second"))),
        &second_answer,
        &mut second_logger,
    )
    .await
    .expect("the second chat dispatch");
    let first_sent = first_answer.requests();
    let second_sent = second_answer.requests();
    assert_eq!(first_sent[0].url, "https://first.test/v1/chat/completions");
    assert_eq!(
        first_sent[0].header("authorization"),
        Some("Bearer sk-first")
    );
    assert_eq!(
        first_sent[0].header("openai-organization"),
        Some("org-first")
    );
    assert_eq!(first_sent[0].header("x-tenant"), Some("first"));
    assert_eq!(
        second_sent[0].url,
        "https://second.test/v1/chat/completions"
    );
    assert_eq!(
        second_sent[0].header("authorization"),
        Some("Bearer sk-second")
    );
    assert_eq!(
        second_sent[0].header("openai-organization"),
        Some("org-second")
    );
    assert_eq!(second_sent[0].header("x-tenant"), Some("second"));

    // a third call without a caller map resolves the configured identity again
    let third_answer = FixtureAnswer::new(200, support::CHAT_REPLY.as_bytes().to_vec());
    let mut third_logger = FixtureLogger::default();
    let _result = support::dispatch_chat(&first, call(None), &third_answer, &mut third_logger)
        .await
        .expect("the third chat dispatch");
    let third_sent = third_answer.requests();
    assert_eq!(
        third_sent[0].header("authorization"),
        Some("Bearer sk-first")
    );
    assert_eq!(third_sent[0].header("x-tenant"), None);
}
