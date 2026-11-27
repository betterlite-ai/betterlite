use betterlite_config::sections::jwt_settings::Settings as JwtAuthSettings;
use betterlite_config::sections::jwt_settings::records::{IssuerConfig, Selector};
use betterlite_config::types::Config;

// JWT_AUDIENCE and JWT_ISSUER join the litellm_jwtauth object B869 already typed; nothing here parses it again.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JwtVerificationSettings {
    pub audience: Option<String>,
    pub issuer: Option<String>,
    pub jwt: JwtAuthSettings,
}

// pyjwt's decode options dict: a key appears only when its check is turned off, and both on is no options at all.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeOptions {
    pub verify_aud: Option<bool>,
    pub verify_iss: Option<bool>,
}

impl JwtVerificationSettings {
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        Self {
            audience: config.sections.jwt_verification.audience.clone(),
            issuer: config.sections.jwt_verification.issuer.clone(),
            jwt: config.sections.jwt_settings.clone(),
        }
    }

    // get_issuer_config: the first exact issuer match wins, and later duplicates are never reached
    #[must_use]
    pub fn issuer_config(&self, issuer: Option<&str>) -> Option<&IssuerConfig> {
        self.jwt.issuer_config(issuer)
    }

    #[must_use]
    pub fn issuers(&self) -> &[IssuerConfig] {
        self.jwt.routing.issuers.as_deref().unwrap_or_default()
    }

    // a configured endpoint that is empty is still configured, and get_oidc_userinfo rejects it before any cache read
    #[must_use]
    pub fn userinfo_endpoint(&self) -> Option<&str> {
        self.jwt.userinfo.oidc_userinfo_endpoint.as_deref()
    }

    #[must_use]
    pub const fn userinfo_ttl(&self) -> f64 {
        self.jwt.userinfo.oidc_userinfo_cache_ttl
    }

    // the state builds a verifier only when some JWT input exists, so a proxy without JWT auth answers as before
    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.audience.is_some()
            || self.issuer.is_some()
            || !self.issuers().is_empty()
            || self.userinfo_endpoint().is_some()
    }
}

// the audience candidates pyjwt compares, one or many
#[must_use]
pub fn audience_candidates(audience: &Selector) -> Vec<String> {
    match audience {
        Selector::One(value) => vec![value.clone()],
        Selector::Many(values) => values.clone(),
    }
}
