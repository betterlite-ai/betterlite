use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;

use crate::jwt_keys::settings::{JwtKeySettings, SettingsError};

use super::support::{settings, settings_error};

fn loaded(yaml: &str, environment: &[(&str, &str)]) -> Config {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }.load(yaml).unwrap()
}

#[test]
fn negative_stale_ttl_validation() {
    assert_eq!(
        settings_error(None, 600.0, -1.0),
        SettingsError::NegativeStaleTtl { value: -1.0_f64 }
    );
    assert_eq!(
        settings_error(None, 600.0, -0.5).to_string(),
        "public_key_stale_ttl must be at least 0, got -0.5"
    );

    // zero is the fail-closed setting, not an error
    let closed = settings(None, 600.0, 0.0);
    assert!(closed.public_key_stale_ttl.total_cmp(&0.0_f64).is_eq());
}

#[test]
fn typed_config_reaches_the_key_settings() {
    let yaml = "
model_list: []
general_settings:
  litellm_jwtauth:
    public_key_ttl: 45.5
    public_key_stale_ttl: 90
    user_allowed_email_domain: example.com
    issuers:
      - issuer: https://idp.example.com
        jwks_url: https://idp.example.com/jwks
        audience: litellm
";
    let raw = " https://idp.example.com/discovery , https://other.example.com/jwks ";
    let config = loaded(yaml, &[("JWT_PUBLIC_KEY_URL", raw)]);
    let settings = JwtKeySettings::from_config(&config);
    assert_eq!(settings.public_key_url.as_deref(), Some(raw));
    assert!(settings.public_key_ttl.total_cmp(&45.5_f64).is_eq());
    assert!(settings.public_key_stale_ttl.total_cmp(&90.0_f64).is_eq());
    assert_eq!(
        settings.user_allowed_email_domain.as_deref(),
        Some("example.com")
    );
    let issuers = settings.issuers.unwrap();
    assert_eq!(issuers.len(), 1);
    assert_eq!(issuers[0].issuer, "https://idp.example.com");
    assert_eq!(
        issuers[0].jwks_url.as_deref(),
        Some("https://idp.example.com/jwks")
    );
}

#[test]
fn absent_keys_keep_litellm_defaults() {
    let config = loaded("model_list: []\n", &[]);
    let settings = JwtKeySettings::from_config(&config);
    assert_eq!(settings.public_key_url, None);
    assert!(settings.public_key_ttl.total_cmp(&600.0_f64).is_eq());
    assert!(
        settings
            .public_key_stale_ttl
            .total_cmp(&3_600.0_f64)
            .is_eq()
    );
    assert_eq!(settings.user_allowed_email_domain, None);
    assert_eq!(settings.issuers, None);
}
