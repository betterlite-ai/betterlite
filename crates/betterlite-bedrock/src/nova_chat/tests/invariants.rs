use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;

use super::support::{json, secret, secret_lookup};
use crate::nova_chat::direct::{self, NovaEndpointRequest};
use crate::nova_chat::error::NovaChatError;
use crate::nova_chat::invoke::{self, InlineRequest};

const SECRET_TEXT: &str = "sk-nova-secret-value";

// Every credential enters through the call, so one call's key never reaches another endpoint and an earlier key never stands in for a later keyless call.
#[test]
fn nova_chat_credential_identity_isolation() {
    let secrets = secret_lookup();
    let first_key = secret("first-key");
    let second_key = secret("second-key");
    let empty = BTreeMap::new();
    let first = direct::resolve_endpoint(
        NovaEndpointRequest {
            api_base: None,
            api_key: Some(&first_key),
            global_api_key: None,
            litellm_api_key: None,
        },
        &empty,
        &secrets,
    )
    .expect("the endpoint resolves");
    assert_eq!(
        first
            .api_key
            .as_ref()
            .expect("the first key")
            .expose_secret(),
        "first-key"
    );
    let second = direct::resolve_endpoint(
        NovaEndpointRequest {
            api_base: None,
            api_key: Some(&second_key),
            global_api_key: None,
            litellm_api_key: None,
        },
        &empty,
        &secrets,
    )
    .expect("the endpoint resolves");
    assert_eq!(
        second
            .api_key
            .as_ref()
            .expect("the second key")
            .expose_secret(),
        "second-key"
    );
    let environment =
        BTreeMap::from([(String::from("AMAZON_NOVA_API_KEY"), String::from("env-key"))]);
    let with_environment = direct::resolve_endpoint(
        NovaEndpointRequest {
            api_base: None,
            api_key: None,
            global_api_key: None,
            litellm_api_key: None,
        },
        &environment,
        &secrets,
    )
    .expect("the endpoint resolves");
    assert_eq!(
        with_environment
            .api_key
            .as_ref()
            .expect("the environment key")
            .expose_secret(),
        "env-key"
    );
    let unkeyed = direct::resolve_endpoint(
        NovaEndpointRequest {
            api_base: None,
            api_key: None,
            global_api_key: None,
            litellm_api_key: None,
        },
        &empty,
        &secrets,
    )
    .expect("the endpoint resolves");
    assert!(unkeyed.api_key.is_none());
}

// A failure names the request member it refused, never the configured credential.
#[test]
fn nova_chat_secret_safe_failure() {
    let secrets = secret_lookup();
    let key = secret(SECRET_TEXT);
    let endpoint = direct::resolve_endpoint(
        NovaEndpointRequest {
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
