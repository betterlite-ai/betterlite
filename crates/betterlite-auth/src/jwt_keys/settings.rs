use betterlite_config::sections::jwt_settings::records::IssuerConfig;
use betterlite_config::types::Config;
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum SettingsError {
    #[error("public_key_stale_ttl must be at least 0, got {value}")]
    NegativeStaleTtl { value: f64 },
}

// The values JWTHandler reads back through its settings block, already typed by the config load.
#[derive(Clone, Debug, PartialEq)]
pub struct JwtKeySettings {
    // None means JWT_PUBLIC_KEY_URL is absent, '' that it was set empty, and the raw text is split per call
    pub public_key_url: Option<String>,
    pub public_key_ttl: f64,
    pub public_key_stale_ttl: f64,
    pub user_allowed_email_domain: Option<String>,
    pub issuers: Option<Vec<IssuerConfig>>,
}

impl JwtKeySettings {
    pub fn new(
        public_key_url: Option<String>,
        public_key_ttl: f64,
        public_key_stale_ttl: f64,
        user_allowed_email_domain: Option<String>,
        issuers: Option<Vec<IssuerConfig>>,
    ) -> Result<Self, SettingsError> {
        if public_key_stale_ttl < 0.0_f64 {
            return Err(SettingsError::NegativeStaleTtl {
                value: public_key_stale_ttl,
            });
        }
        Ok(Self {
            public_key_url,
            public_key_ttl,
            public_key_stale_ttl,
            user_allowed_email_domain,
            issuers,
        })
    }

    // B869 validates litellm_jwtauth as it loads, so a loaded Config never carries a negative stale bound
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        let jwt = &config.sections.jwt_settings;
        Self {
            public_key_url: config.sections.jwt_keys.public_key_url.clone(),
            public_key_ttl: jwt.keys.public_key_ttl,
            public_key_stale_ttl: jwt.keys.public_key_stale_ttl,
            user_allowed_email_domain: jwt.claims.user_allowed_email_domain.clone(),
            issuers: jwt.routing.issuers.clone(),
        }
    }
}
