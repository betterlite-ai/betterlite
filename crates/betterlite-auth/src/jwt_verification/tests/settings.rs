use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::jwt_settings::Settings as JwtAuthSettings;
use betterlite_config::sections::jwt_verification::{AUDIENCE_VARIABLE, ISSUER_VARIABLE};

use crate::jwt_verification::settings::JwtVerificationSettings;

use super::support::{issuer_config, jwt_settings_with_issuers, jwt_settings_with_userinfo};

#[test]
fn loads_the_global_scope_through_the_config_loader() {
    let lookup = BTreeMap::from([
        (AUDIENCE_VARIABLE.to_owned(), "litellm-proxy".to_owned()),
        (
            ISSUER_VARIABLE.to_owned(),
            "https://idp.example.com/".to_owned(),
        ),
    ]);
    let config = ConfigLoader { lookup }.load("model_list: []\n").unwrap();
    let settings = JwtVerificationSettings::from_config(&config);
    assert_eq!(settings.audience.as_deref(), Some("litellm-proxy"));
    assert_eq!(settings.issuer.as_deref(), Some("https://idp.example.com/"));
}

#[test]
fn each_jwt_input_keeps_the_verifier_configured() {
    assert!(!JwtVerificationSettings::default().is_configured());

    let audience = JwtVerificationSettings {
        audience: Some("litellm-proxy".to_owned()),
        issuer: None,
        jwt: JwtAuthSettings::default(),
    };
    assert!(audience.is_configured());

    let issuer = JwtVerificationSettings {
        audience: None,
        issuer: Some("https://idp.example.com".to_owned()),
        jwt: JwtAuthSettings::default(),
    };
    assert!(issuer.is_configured());

    let issuers = JwtVerificationSettings {
        audience: None,
        issuer: None,
        jwt: jwt_settings_with_issuers(vec![issuer_config("https://idp.example.com")]),
    };
    assert!(issuers.is_configured());

    let no_issuers = JwtVerificationSettings {
        audience: None,
        issuer: None,
        jwt: jwt_settings_with_issuers(Vec::new()),
    };
    assert!(!no_issuers.is_configured());

    let userinfo = JwtVerificationSettings {
        audience: None,
        issuer: None,
        jwt: jwt_settings_with_userinfo(Some("https://idp.example.com/userinfo"), 300.0),
    };
    assert!(userinfo.is_configured());
}
