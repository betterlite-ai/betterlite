use secrecy::{ExposeSecret, SecretString};

use crate::vector_stores::environment::{AuthCredentials, Headers, Params, auth_credentials};
use crate::vector_stores::error::AzureSearchError;

fn params(text: &str) -> Params {
    serde_json::from_str(text).expect("fixture params")
}

fn identity(api_key: &str) -> AuthCredentials {
    auth_credentials(&params(&format!(r#"{{"api_key":"{api_key}"}}"#))).expect("credentials")
}

fn header_value(credentials: &AuthCredentials, name: &str) -> String {
    credentials
        .headers
        .get(name)
        .expect("header")
        .expose_secret()
        .to_owned()
}

// two configured identities keep their own keys, and no cache hands one identity the other's header
#[test]
fn foundry_vector_store_credential_identity_isolation() {
    let first = identity("fixture-identity-one");
    let second = identity("fixture-identity-two");
    assert_eq!(header_value(&first, "api-key"), "fixture-identity-one");
    assert_eq!(header_value(&second, "api-key"), "fixture-identity-two");
    assert_ne!(first.headers, second.headers);
    let repeated = identity("fixture-identity-one");
    assert_eq!(repeated, first);
    assert!(
        !repeated
            .headers
            .values()
            .any(|value| value.expose_secret() == "fixture-identity-two")
    );
    let first_url = complete_url(
        None,
        &params(r#"{"azure_search_service_name":"service-one"}"#),
    )
    .expect("url");
    let second_url = complete_url(
        None,
        &params(r#"{"azure_search_service_name":"service-two"}"#),
    )
    .expect("url");
    assert_ne!(first_url, second_url);
    assert!(first_url.contains("service-one"));
}

// Debug and the owned failures name the field or the setting, never the key they were given
#[test]
fn foundry_vector_store_secret_safe_failure() {
    let fixture_secret = "fixture-secret-value";
    let credentials = AuthCredentials {
        headers: Headers::from([("api-key".to_owned(), SecretString::from(fixture_secret))]),
    };
    let debug = format!("{credentials:?}");
    assert!(!debug.contains(fixture_secret));
    assert!(debug.contains("REDACTED"));
    let missing = auth_credentials(&Params::new()).expect_err("missing key");
    assert!(matches!(missing, AzureSearchError::MissingApiKey));
    assert!(!format!("{missing:?}").contains(fixture_secret));
    let invalid = auth_credentials(&params(r#"{"api_key":1}"#)).expect_err("wrong type");
    assert!(!format!("{invalid:?}").contains(fixture_secret));
}
