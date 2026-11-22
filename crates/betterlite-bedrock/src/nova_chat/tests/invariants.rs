use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};

use super::support::{json, secret, secret_lookup};
use crate::nova_chat::direct::{self, NovaEndpointRequest};
use crate::nova_chat::error::NovaChatError;
use crate::nova_chat::invoke::{self, InlineRequest};

const SECRET_TEXT: &str = "sk-nova-secret-value";

fn resolved_key(
    api_key: Option<&SecretString>,
    environment: &BTreeMap<String, String>,
) -> Option<String> {
    let endpoint = direct::resolve_endpoint(
        &NovaEndpointRequest {
            api_base: None,
            api_key,
            global_api_key: None,
            litellm_api_key: None,
        },
        environment,
        &secret_lookup(),
    )
    .expect("the endpoint resolves");
    endpoint.api_key.map(|key| key.expose_secret().to_owned())
}

// Every credential enters through the call, so one call's key never reaches another endpoint and an earlier key never stands in for a later keyless call.
#[test]
fn nova_chat_credential_identity_isolation() {
    let empty = BTreeMap::new();
    let first = resolved_key(Some(&secret("first-key")), &empty);
    assert_eq!(first.as_deref(), Some("first-key"));
    let second = resolved_key(Some(&secret("second-key")), &empty);
    assert_eq!(second.as_deref(), Some("second-key"));
    let environment =
        BTreeMap::from([(String::from("AMAZON_NOVA_API_KEY"), String::from("env-key"))]);
    assert_eq!(resolved_key(None, &environment).as_deref(), Some("env-key"));
    assert_eq!(resolved_key(None, &empty), None);
}

// A failure names the request member it refused, never the configured credential.
#[test]
fn nova_chat_secret_safe_failure() {
    let secrets = secret_lookup();
    let key = secret(SECRET_TEXT);
    let endpoint = direct::resolve_endpoint(
        &NovaEndpointRequest {
            api_base: None,
            api_key: Some(&key),
            global_api_key: None,
            litellm_api_key: None,
        },
        &BTreeMap::new(),
        &secrets,
    )
    .expect("the endpoint resolves");
    assert!(!format!("{endpoint:?}").contains(SECRET_TEXT));
    let messages = vec![json(r#"{"content": []}"#)];
    let error = invoke::inline_cache_points(&InlineRequest {
        system: &[],
        messages: &messages,
    })
    .expect_err("a message without a role is refused");
    assert!(matches!(
        error,
        NovaChatError::BodyField { field: "role", .. }
    ));
    assert!(!format!("{error}").contains(SECRET_TEXT));
    assert!(!format!("{error:?}").contains(SECRET_TEXT));
}
