use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Json;
use secrecy::SecretString;

use crate::responses::auth::{EnvironmentRequest, validate_environment};
use crate::responses::tests::support::{FixtureSources, accessors, environment, headers, object};
use crate::responses::url::complete_url;

pub(super) fn credential_identity_isolation() {
    header_identities();
    entra_identities();
    url_identities();
    repeat_is_stable();
}

fn auth_headers(key: &str, api_base: &str) -> BTreeMap<String, String> {
    let caller = headers(&[]);
    let env = environment(&[]);
    let sources = FixtureSources::default();
    let secret = SecretString::from(key.to_owned());
    validate_environment(&EnvironmentRequest {
        headers: &caller,
        accessors: accessors(None, None, None, None),
        api_base: Some(api_base),
        api_key: Some(&secret),
        params: None,
        environment: &env,
        refresh_enabled: false,
        sources: &sources,
    })
    .unwrap()
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
        accessors: accessors(None, None, None, None),
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
    let second_sources = FixtureSources::default();
    let second = entra_headers(
        &second_sources,
        "tenant-second",
        "client-second",
        "secret-second",
    );
    let first_token = first.get("Authorization").unwrap();
    assert!(first_token.contains("tenant=tenant-first"), "{first_token}");
    assert!(!first_token.contains("secret-second"), "{first_token}");
    let second_token = second.get("Authorization").unwrap();
    assert!(
        second_token.contains("client=client-second"),
        "{second_token}"
    );
    assert!(!second_token.contains("tenant-first"), "{second_token}");
    let first_calls = first_sources.calls.lock().unwrap();
    assert_eq!(first_calls.len(), 1);
    assert!(
        first_calls[0].contains("secret-first"),
        "{}",
        first_calls[0]
    );
    assert!(
        !first_calls[0].contains("client-second"),
        "{}",
        first_calls[0]
    );
    let second_calls = second_sources.calls.lock().unwrap();
    assert_eq!(second_calls.len(), 1);
    assert!(
        !second_calls[0].contains("secret-first"),
        "{}",
        second_calls[0]
    );
}

fn url_identities() {
    let sources = accessors(None, None, None, None);
    let first = complete_url(sources, Some("https://first.services.ai.azure.com")).unwrap();
    let second = complete_url(
        sources,
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
