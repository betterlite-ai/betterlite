use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use betterlite_config::sections::jwt_settings::records::{IssuerConfig, Selector};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::TypedValue;
use super::algorithms::JwtAlgorithm;
use super::claims::{ClaimChecks, validate_claims};
use super::decode::{JwtHeader, decode_signed_jwt};
use super::errors::VerificationError;
use super::keys::{PublicKey, verify_signature};
use super::normalize::{normalize_with_issuer, strip_internal_claims};
use super::settings::{DecodeOptions, JwtVerificationSettings};
use super::source::PublicKeySource;
use super::userinfo::{UserinfoError, UserinfoHttp, fetch_userinfo};
use crate::jwt_keys::cache::KeyCache;
use crate::jwt_keys::clock::Clock;

const DISCOVERY_SUFFIX: &str = "/.well-known/openid-configuration";

// _build_decode_kwargs returns the env audience, the env issuer and pyjwt's options dict, or no options at all.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DecodeKwargs {
    pub audience: Option<String>,
    pub issuer: Option<String>,
    pub options: Option<DecodeOptions>,
}

// JWTHandler: verification, trusted-issuer selection, claim normalization and the OIDC UserInfo path.
pub struct JwtVerifier {
    settings: Arc<JwtVerificationSettings>,
    cache: Arc<dyn KeyCache>,
    http: Arc<dyn UserinfoHttp>,
    clock: Arc<dyn Clock>,
    unscoped_warned: AtomicBool,
}

impl JwtVerifier {
    #[must_use]
    pub fn new(
        settings: Arc<JwtVerificationSettings>,
        cache: Arc<dyn KeyCache>,
        http: Arc<dyn UserinfoHttp>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            settings,
            cache,
            http,
            clock,
            unscoped_warned: AtomicBool::new(false),
        }
    }

    // auth_jwt: parse and log the unverified header, pick a configured issuer, then fetch a key and verify.
    pub async fn auth_jwt(
        &self,
        token: &str,
        source: &dyn PublicKeySource,
    ) -> Result<TypedValue, VerificationError> {
        let header = JwtHeader::parse(token)?;
        tracing::debug!(
            target: "LiteLLM Proxy",
            "header: alg={:?} kid={:?}",
            header.algorithm,
            header.kid
        );
        if let Some(config) = self.configured_issuer(token) {
            return self
                .auth_issuer_jwt(token, config, header.kid.as_deref(), source)
                .await;
        }
        let kwargs = self.build_decode_kwargs();
        let resolved = source.resolve_public_key(header.kid.as_deref()).await?;
        let Some(public_key) = PublicKey::from_resolved(&resolved) else {
            return Err(VerificationError::InvalidJwt);
        };
        let env_audience = kwargs
            .audience
            .as_deref()
            .map(|text| Selector::One(text.to_owned()));
        let payload = self.decode_with_public_key(
            token,
            &public_key,
            env_audience.as_ref(),
            kwargs.issuer.as_deref(),
            kwargs.options,
            false,
        )?;
        Ok(strip_internal_claims(&payload)?)
    }

    // _auth_jwt_with_issuer: the key fetch is outside the wrapping, so an outage keeps its own 503.
    async fn auth_issuer_jwt(
        &self,
        token: &str,
        config: &IssuerConfig,
        kid: Option<&str>,
        source: &dyn PublicKeySource,
    ) -> Result<TypedValue, VerificationError> {
        let jwks_url = Self::jwks_url_for_issuer(config);
        let resolved = source.resolve_keys_for_url(&jwks_url, kid).await?;
        let Some(public_key) = PublicKey::from_resolved(&resolved) else {
            return Err(VerificationError::InvalidJwt);
        };
        let payload = self.decode_with_public_key(
            token,
            &public_key,
            config.audience.as_ref(),
            Some(config.issuer.as_str()),
            None,
            config.disable_audience_validation,
        )?;
        Ok(normalize_with_issuer(&payload, config)?)
    }

    // _build_decode_kwargs: warned once per process when neither JWT_AUDIENCE nor JWT_ISSUER scopes the proxy.
    pub fn build_decode_kwargs(&self) -> DecodeKwargs {
        let audience = self.settings.audience.clone();
        let issuer = self.settings.issuer.clone();
        if audience.is_none()
            && issuer.is_none()
            && !self.unscoped_warned.swap(true, Ordering::SeqCst)
        {
            tracing::warn!(
                target: "LiteLLM Proxy",
                "JWT auth is enabled but neither JWT_AUDIENCE nor JWT_ISSUER is configured. Tokens minted by any application that shares the same IdP signing keys will be accepted. Set JWT_AUDIENCE (and ideally JWT_ISSUER) to scope this proxy."
            );
        }
        let options = if audience.is_some() && issuer.is_some() {
            None
        } else {
            Some(DecodeOptions {
                verify_aud: audience.is_none().then_some(false),
                verify_iss: issuer.is_none().then_some(false),
            })
        };
        DecodeKwargs {
            audience,
            issuer,
            options,
        }
    }

    // managed_issuer_is_trusted: the first matching configured issuer decides, and the env pair is the fallback.
    #[must_use]
    pub fn managed_issuer_is_trusted(&self, issuer: &Json) -> bool {
        let Json::Text(issuer) = issuer else {
            return false;
        };
        for config in self.settings.jwt.routing.issuers.iter().flatten() {
            if config.issuer == *issuer {
                let scoped = match config.audience.as_ref() {
                    Some(Selector::One(value)) => !value.is_empty(),
                    Some(Selector::Many(values)) => !values.is_empty(),
                    None => false,
                };
                return scoped && !config.disable_audience_validation;
            }
        }
        self.settings.issuer.as_deref() == Some(issuer.as_str())
            && self
                .settings
                .audience
                .as_deref()
                .is_some_and(|audience| !audience.is_empty())
    }

    // _get_jwks_url_for_issuer: a truthy jwks_url wins, else the issuer's discovery document.
    #[must_use]
    pub fn jwks_url_for_issuer(config: &IssuerConfig) -> String {
        match config.jwks_url.as_deref() {
            Some(url) if !url.is_empty() => url.to_owned(),
            Some(_) | None => format!("{}{DISCOVERY_SUFFIX}", config.issuer.trim_end_matches('/')),
        }
    }

    // _get_decode_options: disabling audience verification is an explicit choice, never implied by a missing audience.
    pub fn decode_options(
        audience: Option<&Selector>,
        issuer: Option<&str>,
        disable_audience_validation: bool,
    ) -> Result<Option<DecodeOptions>, VerificationError> {
        if audience.is_none() && !disable_audience_validation {
            return Err(VerificationError::Validation {
                detail: "audience must be provided unless disable_audience_validation=True"
                    .to_owned(),
            });
        }
        let options = DecodeOptions {
            verify_aud: audience.is_none().then_some(false),
            verify_iss: issuer.is_none().then_some(false),
        };
        Ok((audience.is_none() || issuer.is_none()).then_some(options))
    }

    // _get_configured_issuer: the first exact config for a nonempty string iss, read from unverified claims.
    fn configured_issuer(&self, token: &str) -> Option<&IssuerConfig> {
        if self
            .settings
            .jwt
            .routing
            .issuers
            .as_deref()
            .unwrap_or_default()
            .is_empty()
        {
            return None;
        }
        let claims = decode_signed_jwt(token.as_bytes()).ok()?;
        let issuer = claims.json.get("iss")?.as_str()?;
        if issuer.is_empty() {
            return None;
        }
        self.settings.jwt.issuer_config(Some(issuer))
    }

    // _decode_jwt_with_public_key: explicit options bypass option construction, then the signature and claims are checked.
    pub fn decode_with_public_key(
        &self,
        token: &str,
        public_key: &PublicKey<'_>,
        audience: Option<&Selector>,
        issuer: Option<&str>,
        options: Option<DecodeOptions>,
        disable_audience_validation: bool,
    ) -> Result<TypedValue, VerificationError> {
        let checks_on = match options {
            Some(explicit) => explicit,
            None => Self::decode_options(audience, issuer, disable_audience_validation)?.unwrap_or(
                DecodeOptions {
                    verify_aud: None,
                    verify_iss: None,
                },
            ),
        };
        let header = JwtHeader::parse(token)?;
        let Some(name) = header.algorithm.as_deref() else {
            return Err(unsupported_algorithm());
        };
        let Some(algorithm) = JwtAlgorithm::from_name(name) else {
            return Err(unsupported_algorithm());
        };
        let Some((signing_input, signature)) = token.rsplit_once('.') else {
            return Err(unsupported_algorithm());
        };
        verify_signature(public_key, algorithm, signing_input.as_bytes(), signature)?;
        let payload = decode_signed_jwt(token.as_bytes())?;
        let checks = ClaimChecks {
            audience,
            issuer,
            verify_aud: checks_on.verify_aud.unwrap_or(true),
            verify_iss: checks_on.verify_iss.unwrap_or(true),
            now: self.clock.now(),
        };
        validate_claims(&payload, &checks)?;
        Ok(payload)
    }

    // get_oidc_userinfo: the separate HTTP identity path, unchanged by any JWKS retry or stale policy.
    pub async fn get_oidc_userinfo(
        &self,
        token: &str,
    ) -> Result<IndexMap<String, Json>, UserinfoError> {
        fetch_userinfo(
            &self.settings,
            self.cache.as_ref(),
            self.http.as_ref(),
            token,
        )
        .await
    }
}

// close: the verifier owns its HTTP handler, so releasing it closes that handler as LiteLLM's close does.
impl Drop for JwtVerifier {
    fn drop(&mut self) {
        self.http.close();
    }
}

fn unsupported_algorithm() -> VerificationError {
    VerificationError::Validation {
        detail: "The specified alg value is not allowed".to_owned(),
    }
}
