use crate::rerank::auth::ValidateEnvironmentRequest;
use crate::rerank::error::FoundryRerankError;

use super::support::{
    FixtureSources, SENTINEL, auth_request, environment, failure, headers, key, object, resolved,
};

pub(super) fn credential_identity_isolation() {
    interleaved_identities();
    no_cross_credential_headers();
    caller_headers_stay_with_their_call();
}

fn interleaved_identities() {
    let sources = FixtureSources::new();
    let empty = environment(&[]);
    let env_second = environment(&[("AZURE_AI_API_KEY", "env-second")]);
    let first = key("key-first");
    let global = key("key-global");
    let token = object(r#"{"azure_ad_token":"token-third"}"#);
    assert_bearer(
        &auth_request(Some(&first), &empty, None, None, &sources),
        "Bearer key-first",
    );
    assert_bearer(
        &auth_request(None, &env_second, None, None, &sources),
        "Bearer env-second",
    );
    assert_bearer(
        &auth_request(Some(&first), &empty, None, None, &sources),
        "Bearer key-first",
    );
    assert_bearer(
        &auth_request(None, &empty, Some(&global), None, &sources),
        "Bearer key-global",
    );
    assert_bearer(
        &auth_request(None, &empty, None, Some(&token), &sources),
        "Bearer token-third",
    );
    assert_bearer(
        &auth_request(None, &empty, Some(&global), None, &sources),
        "Bearer key-global",
    );
    assert!(!sources.took());
}

fn no_cross_credential_headers() {
    let sources = FixtureSources::new();
    let empty = environment(&[]);
    let env_second = environment(&[("AZURE_AI_API_KEY", "env-second")]);
    let first = key("key-first");
    let global = key("key-global");
    let first_headers = resolved(&auth_request(Some(&first), &empty, None, None, &sources));
    assert!(
        !first_headers
            .values()
            .any(|value| value.contains("env-second") || value.contains("key-global"))
    );
    let second_headers = resolved(&auth_request(
        None,
        &env_second,
        Some(&global),
        None,
        &sources,
    ));
    assert_eq!(
        second_headers.get("Authorization").map(String::as_str),
        Some("Bearer env-second")
    );
    assert!(
        !second_headers
            .values()
            .any(|value| value.contains("key-first"))
    );
    assert!(!sources.took());
}

fn caller_headers_stay_with_their_call() {
    let sources = FixtureSources::new();
    let empty = environment(&[]);
    let first = key("key-first");
    let second = key("key-second");
    let mut decorated = auth_request(Some(&first), &empty, None, None, &sources);
    decorated.headers = headers(&[("X-Identity", "first")]);
    assert_eq!(
        resolved(&decorated).get("X-Identity").map(String::as_str),
        Some("first")
    );
    let plain = resolved(&auth_request(Some(&second), &empty, None, None, &sources));
    assert_eq!(plain.get("X-Identity"), None);
    assert_eq!(
        plain.get("Authorization").map(String::as_str),
        Some("Bearer key-second")
    );
    assert!(!sources.took());
}

fn assert_bearer(request: &ValidateEnvironmentRequest<'_>, bearer: &str) {
    assert_eq!(
        resolved(request).get("Authorization").map(String::as_str),
        Some(bearer)
    );
}

pub(super) fn secret_safe_failure() {
    resolved_bearer_is_visible();
    failures_hide_the_secret();
    refused_source_hides_the_secret();
}

// the failure checks are only meaningful while a sentinel that did resolve is shown to resolve
fn resolved_bearer_is_visible() {
    let sources = FixtureSources::new();
    let empty = environment(&[]);
    let sentinel_key = key(SENTINEL);
    let resolved_headers = resolved(&auth_request(
        Some(&sentinel_key),
        &empty,
        None,
        None,
        &sources,
    ));
    let bearer = format!("Bearer {SENTINEL}");
    assert_eq!(
        resolved_headers.get("Authorization").map(String::as_str),
        Some(bearer.as_str())
    );
    assert!(!sources.took());
}

fn failures_hide_the_secret() {
    let sources = FixtureSources::new();
    let empty = environment(&[]);
    let unresolved = object(&format!(r#"{{"client_secret":"{SENTINEL}"}}"#));
    assert_hidden(&failure(&auth_request(
        None,
        &empty,
        None,
        Some(&unresolved),
        &sources,
    )));
    let bad_tenant = object(&format!(
        r#"{{"tenant_id":5,"client_secret":"{SENTINEL}"}}"#
    ));
    assert_hidden(&failure(&auth_request(
        None,
        &empty,
        None,
        Some(&bad_tenant),
        &sources,
    )));
    assert!(!sources.took());
}

fn refused_source_hides_the_secret() {
    let sources = FixtureSources::new();
    let empty = environment(&[]);
    let identity = object(&format!(
        r#"{{"tenant_id":"tenant","client_id":"client","client_secret":"{SENTINEL}"}}"#
    ));
    let refused = failure(&auth_request(None, &empty, None, Some(&identity), &sources));
    assert!(refused.to_string().contains("offline in this fixture"));
    assert_hidden(&refused);
    assert!(sources.took());
}

fn assert_hidden(error: &FoundryRerankError) {
    assert!(!error.to_string().contains(SENTINEL));
    assert!(!format!("{error:?}").contains(SENTINEL));
}
