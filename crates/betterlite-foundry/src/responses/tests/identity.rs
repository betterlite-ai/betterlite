use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Json;
use secrecy::SecretString;

use crate::responses::auth::{EnvironmentRequest, validate_environment};
use crate::responses::tests::support::{
    FixtureSources, environment, headers, headers_for, no_accessors, object,
};
use crate::responses::url::complete_url;

pub(super) fn credential_identity_isolation() {
    header_identities();
    entra_identities();
    url_identities();
    repeat_is_stable();
}

fn auth_headers(key: &str, api_base: &str) -> BTreeMap<String, String> {
    let caller = headers(&[]);
    let secret = SecretString::from(key.to_owned());
    headers_for(&caller, no_accessors(), Some(api_base), Some(&secret))
}

fn header_identities() {
    let first = auth_headers("key-first", "https://first.services.ai.azure.com");
    let second = auth_headers("key-second", "https://second.services.ai.azure.com");
    assert_eq!(first.get("api-key").map(String::as_str), Some("key-first"));
    assert_eq!(
        second.get("api-key").map(String::as_str),
        Some("key-second")
    );
    assert!(!first.values().any(|value| value.contains("key-second")));
    assert!(!second.values().any(|value| value.contains("key-first")));
}

fn entra_headers(
    sources: &FixtureSources,
    tenant: &str,
    client: &str,
    secret: &str,
) -> BTreeMap<String, String> {
    let caller = headers(&[]);
    let env = environment(&[]);
    let params = object([
        ("tenant_id", Json::Text(tenant.to_owned())),
        ("client_id", Json::Text(client.to_owned())),
        ("client_secret", Json::Text(secret.to_owned())),
    ]);
    validate_environment(&EnvironmentRequest {
        headers: &caller,
        accessors: no_accessors(),
        api_base: Some("https://res.services.ai.azure.com"),
        api_key: None,
        params: Some(&params),
        environment: &env,
        refresh_enabled: false,
        sources,
    })
    .unwrap()
}

fn entra_identities() {
    let first_sources = FixtureSources::default();
    let first = entra_headers(
        &first_sources,
        "tenant-first",
        "client-first",
        "secret-first",
    );
    let first_token = &first["Authorization"];
    assert!(first_token.contains("tenant=tenant-first"), "{first_token}");
    assert!(!first_token.contains("secret-second"), "{first_token}");
    assert_single_call(&first_sources, "secret-first", "client-second");
    let second_sources = FixtureSources::default();
    let second = entra_headers(
        &second_sources,
        "tenant-second",
        "client-second",
        "secret-second",
    );
    let second_token = &second["Authorization"];
    assert!(
        second_token.contains("client=client-second"),
        "{second_token}"
    );
    assert!(!second_token.contains("tenant-first"), "{second_token}");
    assert_single_call(&second_sources, "client-second", "secret-first");
}

fn assert_single_call(sources: &FixtureSources, expected: &str, forbidden: &str) {
    let calls = sources.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].contains(expected), "{}", calls[0]);
    assert!(!calls[0].contains(forbidden), "{}", calls[0]);
}

fn url_identities() {
    let first = complete_url(no_accessors(), Some("https://first.services.ai.azure.com")).unwrap();
    let second = complete_url(
        no_accessors(),
        Some("https://second.services.ai.azure.com?api-version=1"),
    )
    .unwrap();
    assert_eq!(
        first,
        "https://first.services.ai.azure.com/openai/v1/responses"
    );
    assert_eq!(
        second,
        "https://second.services.ai.azure.com/openai/v1/responses"
    );
    assert!(!first.contains("second"));
    assert!(!second.contains("first"));
    assert!(!second.contains('?'));
}

fn repeat_is_stable() {
    let first = auth_headers("key-first", "https://first.services.ai.azure.com");
    let second = auth_headers("key-second", "https://second.services.ai.azure.com");
    let again = auth_headers("key-first", "https://first.services.ai.azure.com");
    assert_ne!(first, second);
    assert_eq!(first, again);
}
