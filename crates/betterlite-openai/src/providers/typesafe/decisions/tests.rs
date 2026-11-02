#![cfg(test)]

use super::{DecisionsProviderConfig as _, TypeSafeDecisionsConfig};

#[test]
fn typesafe_config_uses_typesafe_api_key_env_name() {
    assert_eq!(TypeSafeDecisionsConfig::API_KEY_ENV, ["TYPESAFE_API_KEY"]);
}

#[test]
fn typesafe_config_uses_typesafe_api_base_env_name() {
    assert_eq!(TypeSafeDecisionsConfig::API_BASE_ENV, ["TYPESAFE_API_BASE"]);
}

#[test]
fn typesafe_config_defaults_the_api_base_to_the_typesafe_endpoint() {
    let config = TypeSafeDecisionsConfig;
    assert_eq!(
        config.get_default_api_base(),
        Some("https://api.typesafe.ai".to_owned())
    );
}

#[test]
fn typesafe_config_resolves_api_base_from_default_when_unset() {
    let config = TypeSafeDecisionsConfig;
    let get_secret_str = |_: &str| String::new();
    assert_eq!(
        config.resolve_api_base(None, get_secret_str),
        Some("https://api.typesafe.ai".to_owned())
    );
}

#[test]
fn typesafe_config_resolves_api_base_from_its_own_env_name() {
    let config = TypeSafeDecisionsConfig;
    let get_secret_str = |name: &str| {
        if name == "TYPESAFE_API_BASE" {
            "https://custom.example".to_owned()
        } else {
            String::new()
        }
    };
    assert_eq!(
        config.resolve_api_base(None, get_secret_str),
        Some("https://custom.example".to_owned())
    );
}

#[test]
fn typesafe_config_requires_an_api_key() {
    let required: bool = TypeSafeDecisionsConfig::API_KEY_REQUIRED;
    assert!(required);
}

#[test]
fn typesafe_config_does_not_support_safety_identifier() {
    let supports_safety_identifier: bool = TypeSafeDecisionsConfig::SUPPORTS_SAFETY_IDENTIFIER;
    assert!(!supports_safety_identifier);
}

#[test]
fn typesafe_config_resolves_api_key_from_its_own_env_name() {
    let config = TypeSafeDecisionsConfig;
    let get_secret_str = |name: &str| {
        if name == "TYPESAFE_API_KEY" {
            "secret".to_owned()
        } else {
            String::new()
        }
    };
    assert_eq!(
        config.resolve_api_key(None, get_secret_str),
        Some("secret".to_owned())
    );
}
