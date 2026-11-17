use betterlite_openai::openai_base_config::Headers;
use secrecy::ExposeSecret;

use super::support;
use crate::openai_dispatch::call::{ChatCall, Composition};
use crate::openai_dispatch::compose::resolve;
use crate::openai_dispatch::fixtures::answer::{FixtureAnswer, RecordedRequest};
use crate::openai_dispatch::fixtures::logger::FixtureLogger;

fn call(headers: Option<Headers>) -> ChatCall {
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

fn tenant_headers(tenant: &str) -> Headers {
    Headers::from([(String::from("x-tenant"), tenant.to_owned())])
}

fn identity(api_base: &str, api_key: &str, organization: &str) -> Composition {
    let mut composition = support::composition();
    let resolved = support::credentials(Some(api_base), Some(api_key), Some(organization));
    composition.globals = support::globals(resolved, None);
    composition
}

// Two configured identities resolve their own base, key and organization.
fn identities_resolve_apart() {
    let plain = support::credentials(None, None, None);
    let resolved_first = resolve(
        &identity("https://first.test/v1", "sk-first", "org-first"),
        &plain,
    );
    let resolved_second = resolve(
        &identity("https://second.test/v1", "sk-second", "org-second"),
        &plain,
    );
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
}

// An explicit identity on one call is never memoized into the composition or the next call.
fn explicit_identity_is_not_memoized() {
    let first = identity("https://first.test/v1", "sk-first", "org-first");
    let explicit = support::credentials(
        Some("https://explicit.test/v1"),
        Some("sk-explicit"),
        Some("org-explicit"),
    );
    let resolved_explicit = resolve(&first, &explicit);
    assert_eq!(resolved_explicit.api_base, "https://explicit.test/v1");
    let resolved_again = resolve(&first, &support::credentials(None, None, None));
    assert_eq!(resolved_again.api_base, "https://first.test/v1");
    assert_eq!(
        resolved_again
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("sk-first")
    );
}

async fn dispatched(composition: &Composition, headers: Option<Headers>) -> Vec<RecordedRequest> {
    let answer = FixtureAnswer::new(200, support::CHAT_REPLY.as_bytes().to_vec());
    let mut logger = FixtureLogger::default();
    let _result = support::dispatch_chat(composition, call(headers), &answer, &mut logger)
        .await
        .expect("the chat dispatch");
    answer.requests()
}

// Each call's headers and identity reach only its own wire request.
async fn identities_send_their_own_headers() {
    let first = identity("https://first.test/v1", "sk-first", "org-first");
    let first_sent = dispatched(&first, Some(tenant_headers("first"))).await;
    let second = identity("https://second.test/v1", "sk-second", "org-second");
    let second_sent = dispatched(&second, Some(tenant_headers("second"))).await;
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
}

// A third call without a caller map resolves the configured identity again.
async fn a_plain_call_has_no_caller_headers() {
    let first = identity("https://first.test/v1", "sk-first", "org-first");
    let sent = dispatched(&first, None).await;
    assert_eq!(sent[0].header("authorization"), Some("Bearer sk-first"));
    assert_eq!(sent[0].header("x-tenant"), None);
}

#[tokio::test]
async fn openai_provider_credential_identity_isolation() {
    identities_resolve_apart();
    explicit_identity_is_not_memoized();
    identities_send_their_own_headers().await;
    a_plain_call_has_no_caller_headers().await;
}
