use std::time::SystemTime;

use betterlite_config::sections::panw_prisma_airs::{Configured, FallbackAction};
use betterlite_config::tls::TlsPolicy;
use betterlite_http::proxy::base_builder;
use betterlite_http::tls::{TlsError, apply_policy};
use reqwest::header::{
    ACCEPT, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, InvalidHeaderValue,
};
use reqwest::redirect::Policy;
use secrecy::ExposeSecret as _;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConstructError {
    #[error(
        "PANW Prisma AIRS: api_key is required. Set it via config or PANW_PRISMA_AIRS_API_KEY environment variable."
    )]
    MissingApiKey,
    #[error("the PANW Prisma AIRS api_key is not valid in an HTTP header: {source}")]
    InvalidApiKey {
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("could not apply the TLS policy to the PANW Prisma AIRS HTTP client: {source}")]
    Tls {
        #[source]
        source: TlsError,
    },
    #[error("could not build the PANW Prisma AIRS HTTP client: {source}")]
    Client {
        #[source]
        source: reqwest::Error,
    },
}

// Mirrors LiteLLM's PanwPrismaAirsHandler.__init__; the config crate already resolved the env fallbacks, masking flags and app_name prefix.
#[derive(Debug, Clone)]
pub struct PanwPrismaAirs {
    pub(crate) settings: Configured,
    pub(crate) client: reqwest::Client,
    pub(crate) headers: HeaderMap,
    pub(crate) now: fn() -> SystemTime,
}

impl PanwPrismaAirs {
    // Scan calls go out with follow_redirects=False, which reqwest only offers per client, and with the resolved ssl_verify, SSL_CERT_FILE, SSL_CERTIFICATE and LITELLM_USER_AGENT policy (docs/deviations/B3786.yaml).
    pub fn new(settings: Configured, tls: &TlsPolicy) -> Result<Self, ConstructError> {
        if settings.api_key.expose_secret().is_empty() {
            return Err(ConstructError::MissingApiKey);
        }
        if settings.profile_name.is_none() {
            tracing::warn!(
                guardrail = settings.guardrail_name,
                "PANW Prisma AIRS Guardrail: No profile_name configured. Ensure your API key has a linked profile in Strata Cloud Manager, or provide 'profile_name'/'profile_id' via config or per-request metadata. Requests will fail if the API key is not linked to a profile."
            );
        }
        if settings.fallback_on_error == FallbackAction::Allow {
            tracing::warn!(
                guardrail = settings.guardrail_name,
                "PANW Prisma AIRS Guardrail: fallback_on_error='allow' - requests will proceed without scanning when API is unavailable."
            );
        }
        let headers = auth_headers(&settings)?;
        let builder =
            apply_policy(base_builder(), tls).map_err(|source| ConstructError::Tls { source })?;
        let client = builder
            .redirect(Policy::none())
            .build()
            .map_err(|source| ConstructError::Client { source })?;
        tracing::info!(
            guardrail = settings.guardrail_name,
            profile = settings.profile_name.as_deref().unwrap_or("API-key-linked"),
            mask_request = settings.masking.content.request_content,
            mask_response = settings.masking.content.response_content,
            fallback_on_error = ?settings.fallback_on_error,
            timeout = settings.timeout,
            "Initialized PANW Prisma AIRS Guardrail"
        );
        Ok(Self {
            settings,
            client,
            headers,
            now: SystemTime::now,
        })
    }
}

fn auth_headers(settings: &Configured) -> Result<HeaderMap, ConstructError> {
    let mut token = HeaderValue::from_str(settings.api_key.expose_secret())
        .map_err(|source| ConstructError::InvalidApiKey { source })?;
    token.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(HeaderName::from_static("x-pan-token"), token);
    Ok(headers)
}
