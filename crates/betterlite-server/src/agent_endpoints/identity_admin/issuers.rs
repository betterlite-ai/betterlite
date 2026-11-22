use betterlite_config::sections::jwt_settings::records::{IssuerConfig, Selector};

mod tests;

// LiteLLM's litellm_jwtauth.issuers plus the globals, read once by the server from config and env
#[derive(Clone, Debug, Default)]
pub struct IdentitySettings {
    pub enable_jwt_auth: bool,
    pub issuers: Vec<IssuerConfig>,
    pub jwt_issuer: Option<String>,
    pub jwt_audience: Option<String>,
    pub microsoft_tenant: Option<String>,
    pub microsoft_client_id: Option<String>,
}

#[must_use]
pub fn trusted_agent_issuers(settings: &IdentitySettings) -> Vec<String> {
    if !settings.enable_jwt_auth {
        return Vec::new();
    }
    let mut trusted: Vec<String> = settings
        .issuers
        .iter()
        .filter(|entry| {
            let audience_set = match &entry.audience {
                Some(Selector::One(audience)) => !audience.is_empty(),
                Some(Selector::Many(audiences)) => !audiences.is_empty(),
                None => false,
            };
            audience_set && !entry.disable_audience_validation
        })
        .map(|entry| entry.issuer.clone())
        .collect();
    if let Some(issuer) = global_issuer(settings) {
        trusted.push(issuer);
    }
    trusted
}

// JWT_ISSUER counts only with JWT_AUDIENCE, and only when no configured entry names that issuer already
fn global_issuer(settings: &IdentitySettings) -> Option<String> {
    let issuer = settings
        .jwt_issuer
        .as_deref()
        .filter(|issuer| !issuer.is_empty())?;
    let audience_set = settings
        .jwt_audience
        .as_deref()
        .is_some_and(|audience| !audience.is_empty());
    if !audience_set || settings.issuers.iter().any(|entry| entry.issuer == issuer) {
        return None;
    }
    Some(issuer.to_owned())
}
