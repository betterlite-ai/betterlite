use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::gdc_settings::{BoolOrString, CallSettings, GlobalSettings};
use betterlite_openai::gdc_policy;
use betterlite_prompts::arguments::json::Object;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

use crate::gdc_auth::cache::{CacheKey, CredentialIdentity, TokenCache, is_valid};
use crate::gdc_auth::credential::{GdchCredentialFactory, GdchServiceAccount, GdchSigner};
use crate::gdc_auth::error::GdcAuthError;
use crate::gdc_auth::exchange::{Exchange, ExchangeError, ExchangeRequest};
use crate::gdc_auth::headers::{self, Headers};
use crate::gdc_auth::key::{GdcKey, decode_gdc_key};
use crate::gdc_auth::token::{TokenReply, decode_gdc_token_reply};
use crate::gdc_auth::transport::{
    CaCertificates, GdcClock, GdcJitter, GdcSleeper, TokenTransport, TokenTrust,
};

const TOKEN_EXCHANGE_GRANT: &str = "urn:ietf:params:oauth:token-type:token-exchange";
const ACCESS_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:access_token";
const SERVICE_ACCOUNT_TOKEN_TYPE: &str = "urn:k8s:params:oauth:token-type:serviceaccount";

#[rustfmt::skip]
pub struct GdcAuthParts {
    pub credentials: Arc<dyn GdchCredentialFactory>, pub transport: Arc<dyn TokenTransport>, pub ca: Arc<dyn CaCertificates>,
    pub clock: Arc<dyn GdcClock>, pub sleeper: Arc<dyn GdcSleeper>, pub jitter: Arc<dyn GdcJitter>,
}
#[rustfmt::skip]
pub struct GdcAuth { pub parts: GdcAuthParts, pub cache: TokenCache }
#[rustfmt::skip]
pub struct GdcValidation<'value> {
    pub model: &'value str, pub api_base: Option<&'value str>, pub api_key: Option<&'value SecretString>, pub headers: Headers,
    pub optional_params: &'value Object, pub call: &'value CallSettings, pub global: &'value GlobalSettings,
    pub ssl_verify: BoolOrString, pub caching: bool, pub deadline: Duration, pub source_identity: &'value str,
}
#[rustfmt::skip]
struct TokenRequest<'value> {
    api_key: Option<&'value SecretString>, source_identity: &'value str, ssl_verify: &'value BoolOrString,
    caching: bool, deadline: Duration, model: &'value str,
}
#[rustfmt::skip]
#[derive(Serialize)]
struct ExchangeBody<'value> {
    grant_type: &'static str, audience: &'value str, requested_token_type: &'static str,
    subject_token: &'value str, subject_token_type: &'static str,
}
#[rustfmt::skip]
struct Validated<'value> { api_key: &'value SecretString, project: String, audience: String, key: GdcKey }

#[rustfmt::skip]
fn validate_inputs<'value>(request: &GdcValidation<'value>) -> Result<Validated<'value>, GdcAuthError> {
    let model = request.model;
    let base = request.api_base.filter(|base| !base.is_empty()).ok_or_else(|| GdcAuthError::Missing { what: "base URL", model: model.to_owned() })?;
    let api_key = request.api_key.filter(|key| !key.expose_secret().is_empty()).ok_or_else(|| GdcAuthError::Missing { what: "API key", model: model.to_owned() })?;
    let requested_project = gdc_policy::effective_project(base, request.call, request.global, request.optional_params).ok_or_else(|| GdcAuthError::Missing { what: "project", model: model.to_owned() })?;
    let project = gdc_policy::validate_path_id(&requested_project, "vertex_project", model)?.to_owned();
    let audience = gdc_policy::audience(base)?;
    let key = decode_gdc_key(api_key.expose_secret().as_bytes(), model)?;
    Ok(Validated { api_key, project, audience, key })
}

#[rustfmt::skip]
impl GdcAuth {
    pub async fn validate_environment(&self, request: GdcValidation<'_>) -> Result<Headers, GdcAuthError> {
        let validated = validate_inputs(&request)?;
        let token_request = TokenRequest { api_key: Some(validated.api_key), source_identity: request.source_identity, ssl_verify: &request.ssl_verify, caching: request.caching, deadline: request.deadline, model: request.model };
        let mut headers = request.headers;
        match validated.key {
            GdcKey::Bearer(token) => headers::set_bearer_token(&mut headers, &token),
            GdcKey::ServiceAccount(descriptor) => {
                let credential = self.parts.credentials.build(descriptor).map_err(|source| GdcAuthError::Credential { model: request.model.to_owned(), source })?;
                let token = self.token(&credential, &validated.audience, &token_request).await?;
                headers::set_service_token(&mut headers, &token);
            }
        }
        headers::set_content_type(&mut headers);
        headers::set_quota_project(&mut headers, &validated.project);
        Ok(headers)
    }
    async fn token(&self, credential: &Arc<GdchServiceAccount>, audience: &str, request: &TokenRequest<'_>) -> Result<SecretString, GdcAuthError> {
        if request.caching {
            return self.cached_token(credential, audience, request).await;
        }
        Ok(self.refresh(credential, audience, request).await?.token)
    }
    async fn cached_token(&self, credential: &Arc<GdchServiceAccount>, audience: &str, request: &TokenRequest<'_>) -> Result<SecretString, GdcAuthError> {
        let bound = audience.trim_end_matches('/');
        let key = CacheKey { audience: bound.to_owned(), identity: credential_identity(request.api_key, request.source_identity) };
        let _lease = self.cache.acquire().await.map_err(|source| GdcAuthError::Cache { model: request.model.to_owned(), source })?;
        let state = self.cache.state(&key);
        let mut cached = state.token;
        let fresh = is_valid(cached.as_ref(), state.expires_at, self.parts.clock.unix_seconds());
        if fresh && let Some(token) = cached.take() {
            return Ok(token);
        }
        let reply = self.refresh(credential, bound, request).await?;
        let expires_at = reply.expires_in.map(|seconds| self.parts.clock.unix_seconds().saturating_add(seconds));
        self.cache.store(&key, reply.token.clone(), expires_at);
        Ok(reply.token)
    }
    async fn refresh(&self, credential: &GdchServiceAccount, audience: &str, request: &TokenRequest<'_>) -> Result<TokenReply, GdcAuthError> {
        if let GdchSigner::Fixture(token) = &credential.signer {
            return Ok(TokenReply { token: token.clone(), expires_in: None });
        }
        let trust = self.trust(credential, request.ssl_verify, request.model).await?;
        let body = self.exchange_body(credential, audience, request)?;
        self.exchange_reply(credential, body, trust, request).await
    }
    fn exchange_body(&self, credential: &GdchServiceAccount, audience: &str, request: &TokenRequest<'_>) -> Result<String, GdcAuthError> {
        let subject = credential.sign(self.parts.clock.unix_seconds()).map_err(|source| GdcAuthError::Credential { model: request.model.to_owned(), source })?;
        let body = ExchangeBody { grant_type: TOKEN_EXCHANGE_GRANT, audience, requested_token_type: ACCESS_TOKEN_TYPE, subject_token: &subject, subject_token_type: SERVICE_ACCOUNT_TOKEN_TYPE };
        let encoded = serde_json::to_string(&body);
        encoded.map_err(|_error| GdcAuthError::Exchange { model: request.model.to_owned(), source: ExchangeError::Body })
    }
    async fn exchange_reply(&self, credential: &GdchServiceAccount, body: String, trust: TokenTrust, request: &TokenRequest<'_>) -> Result<TokenReply, GdcAuthError> {
        let exchange = Exchange { transport: &*self.parts.transport, sleeper: &*self.parts.sleeper, jitter: &*self.parts.jitter, clock: &*self.parts.clock };
        let deadline = u64::try_from(request.deadline.as_millis()).unwrap_or(u64::MAX);
        let response = exchange.run(ExchangeRequest { url: credential.descriptor.token_uri.clone(), body, trust }, deadline).await.map_err(|source| GdcAuthError::Exchange { model: request.model.to_owned(), source })?;
        decode_gdc_token_reply(&response.body).map_err(|source| GdcAuthError::Exchange { model: request.model.to_owned(), source: ExchangeError::Reply { source } })
    }
    async fn trust(&self, credential: &GdchServiceAccount, ssl_verify: &BoolOrString, model: &str) -> Result<TokenTrust, GdcAuthError> {
        if let Some(path) = credential.descriptor.ca_cert_path.as_deref() {
            return self.load_ca(path, model).await;
        }
        match ssl_verify {
            BoolOrString::Bool(true) => Ok(TokenTrust::System),
            BoolOrString::Bool(false) => Ok(TokenTrust::Insecure),
            BoolOrString::Text(path) => self.load_ca(path, model).await,
        }
    }
    async fn load_ca(&self, path: &str, model: &str) -> Result<TokenTrust, GdcAuthError> {
        let loaded = self.parts.ca.load(path).await.map_err(|source| GdcAuthError::Exchange { model: model.to_owned(), source: ExchangeError::Ca { source } })?;
        let certificate = Arc::from(loaded);
        Ok(TokenTrust::Certificate(certificate))
    }
}

// The cache key uses the exact configured key, or the injected source handle when no key was supplied
#[must_use]
pub fn credential_identity(
    api_key: Option<&SecretString>,
    source_identity: &str,
) -> CredentialIdentity {
    api_key.map_or_else(
        || CredentialIdentity::SourceHandle(source_identity.to_owned()),
        |key| CredentialIdentity::ApiKey(key.clone()),
    )
}
