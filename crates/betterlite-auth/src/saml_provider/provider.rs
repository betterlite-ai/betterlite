use std::sync::Arc;

use betterlite_config::saml_provider::SamlProviderSettings;
use sha2::{Digest as _, Sha256};

use super::authn_request::{authn_request_xml, generate_request_id, redirect_location};
use super::cache::{AUTHN_REQUEST_PREFIX, AUTHN_TTL, IDP_SETTINGS_PREFIX, IDP_TTL, SamlCache};
use super::error::SamlProviderError;
use super::fetch::MetadataFetcher;
use super::idp_metadata::{IdpSettings, extract::decode_saml_metadata};
use super::settings::{MergedSettings, merged_settings};
use super::sp_metadata::{build_sp_metadata, validate_sp_metadata};
use super::verify::{SamlResponseVerifier, SamlVerificationError, VerifiedResponse, VerifyRequest};

pub type SamlClock = Arc<dyn Fn() -> i64 + Send + Sync>;

// the URLs the server adapter derived from the request and the public base url
pub struct ProviderUrls {
    pub metadata_url: String,
    pub acs_url: String,
}

pub struct LoginRedirect {
    pub location: String,
    pub request_id: String,
}

pub struct SamlProvider {
    pub settings: SamlProviderSettings,
    pub cache: Arc<dyn SamlCache>,
    pub fetcher: Arc<dyn MetadataFetcher>,
    pub verifier: Arc<dyn SamlResponseVerifier>,
    pub clock: SamlClock,
    pub backend_available: bool,
}

impl SamlProvider {
    pub async fn login(
        &self,
        urls: &ProviderUrls,
        relay_state: &str,
    ) -> Result<LoginRedirect, SamlProviderError> {
        let settings = self.prepared_settings(urls, false).await?;
        let now = (self.clock)();
        let request_id = generate_request_id();
        let request_xml = authn_request_xml(&settings, &request_id, now)?;
        let sso_url = settings.idp.sso_url.as_deref().unwrap_or_default();
        let location = redirect_location(sso_url, &request_xml, relay_state)?;
        let key = format!("{AUTHN_REQUEST_PREFIX}:{request_id}");
        if let Err(error) = self.cache.put_authn_request(&key, AUTHN_TTL).await {
            tracing::warn!(%error, "Failed to store the SAML AuthnRequest id");
        }
        Ok(LoginRedirect {
            location,
            request_id,
        })
    }

    pub async fn sp_metadata(&self, urls: &ProviderUrls) -> Result<String, SamlProviderError> {
        let settings = self.prepared_settings(urls, true).await?;
        let now = (self.clock)();
        let metadata = build_sp_metadata(&settings, now)?;
        let errors = validate_sp_metadata(&metadata, now);
        if errors.codes.is_empty() {
            return Ok(metadata);
        }
        Err(SamlProviderError::InvalidMetadata(errors))
    }

    pub async fn verify_response(
        &self,
        request: &VerifyRequest<'_>,
    ) -> Result<VerifiedResponse, SamlVerificationError> {
        if !self.backend_available {
            return Err(SamlVerificationError::Unavailable);
        }
        self.verifier.verify(request).await
    }

    async fn prepared_settings(
        &self,
        urls: &ProviderUrls,
        sp_validation_only: bool,
    ) -> Result<MergedSettings, SamlProviderError> {
        if !self.backend_available {
            return Err(SamlProviderError::Unavailable);
        }
        let idp = self.load_idp_settings().await?;
        let settings = merged_settings(&self.settings, &idp, &urls.metadata_url, &urls.acs_url);
        if let Err(errors) = settings.validate(sp_validation_only) {
            return Err(if sp_validation_only {
                SamlProviderError::InvalidMetadataSettings(errors)
            } else {
                SamlProviderError::InvalidSettings(errors)
            });
        }
        Ok(settings)
    }

    async fn load_idp_settings(&self) -> Result<IdpSettings, SamlProviderError> {
        let source = match (
            self.settings.idp_metadata_url.as_deref(),
            self.settings.idp_metadata_xml.as_deref(),
        ) {
            (Some(url), _) if !url.is_empty() => url,
            (_, Some(xml)) => xml,
            _ => return Err(SamlProviderError::Unconfigured),
        };
        let digest = hex::encode(Sha256::digest(source.as_bytes()));
        let key = format!("{IDP_SETTINGS_PREFIX}:{digest}");
        match self.cache.get_idp_settings(&key).await {
            Ok(Some(cached)) => return Ok(cached),
            Ok(None) => {}
            Err(error) => tracing::warn!(%error, "Failed to read cached SAML IdP settings"),
        }
        let Some(idp) = self.parse_metadata().await?.filter(|idp| !idp.is_empty()) else {
            return Err(SamlProviderError::NoUsableIdp);
        };
        if let Err(error) = self.cache.put_idp_settings(&key, &idp, IDP_TTL).await {
            tracing::warn!(%error, "Failed to cache SAML IdP settings");
        }
        Ok(idp)
    }

    async fn parse_metadata(&self) -> Result<Option<IdpSettings>, SamlProviderError> {
        let Some(url) = self.settings.idp_metadata_url.as_deref() else {
            let xml = self
                .settings
                .idp_metadata_xml
                .as_deref()
                .unwrap_or_default();
            return decode_saml_metadata(xml.as_bytes()).map_err(SamlProviderError::Parse);
        };
        let body = self
            .fetcher
            .fetch(url, self.settings.idp_metadata_validate_cert.0)
            .await
            .map_err(SamlProviderError::Fetch)?;
        decode_saml_metadata(&body).map_err(SamlProviderError::Parse)
    }
}
