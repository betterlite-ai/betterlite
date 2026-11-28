use std::sync::Arc;

use betterlite_auth::jwt_claims::decode::unverified_claims;
use betterlite_auth::resolved_key::credential::is_jwt;
use betterlite_auth::session_token_codec::builders::SessionTokenCodec;
use betterlite_db::encryption_scan::V2_GCM_PREFIX;
use betterlite_mcp::envelope::EnvelopePrefix as _;
use betterlite_models::key_digest::hashing::hash_token;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, StoredCipher};
use http::HeaderMap;
use http::header::AUTHORIZATION;
use secrecy::ExposeSecret as _;
use subtle::ConstantTimeEq as _;

use super::presented_key::{LITELLM_KEY_HEADER, litellm_key_from_request};
use super::{ActiveSubjects, IdentityLookup, SharedNonces, SubjectStore};

mod tests;

impl<S: SubjectStore> ActiveSubjects<S> {
    // The browser authorize step asks this before it trusts a bearer, so a denied credential can't use the cookie
    pub async fn uses_gateway_credential(&self, headers: &HeaderMap) -> bool {
        if headers.contains_key(LITELLM_KEY_HEADER) {
            return true;
        }
        let Some(token) = litellm_key_from_request(headers) else {
            return headers.contains_key(AUTHORIZATION);
        };
        if token.starts_with("sk-") || self.matches_master_key(&token) {
            return true;
        }
        if self.settings.custom_auth || self.settings.oidc_userinfo_enabled {
            return true;
        }
        if !is_jwt(Some(&token)) {
            return self.opaque_bearer_credential(&token).await;
        }
        self.jwt_issuer_is_trusted(&token)
    }

    // The comparison is over the encoded bytes, the way secrets.compare_digest reads them
    fn matches_master_key(&self, token: &str) -> bool {
        let Some(master_key) = self.settings.master_key.as_ref() else {
            return false;
        };
        let master_text = master_key.expose_secret();
        if master_text.is_empty() {
            return false;
        }
        bool::from(token.as_bytes().ct_eq(master_text.as_bytes()))
    }

    // The unscoped global validator can accept issuers the configured list doesn't name
    fn jwt_issuer_is_trusted(&self, token: &str) -> bool {
        let Some(claims) = unverified_claims(token) else {
            return true;
        };
        let Some(issuer) = claims.json.get("iss").and_then(|value| value.as_str()) else {
            return true;
        };
        if issuer.is_empty() {
            return true;
        }
        let Some(global_issuer) = self.settings.global_issuer.as_deref() else {
            return true;
        };
        if global_issuer.is_empty() {
            return true;
        }
        issuer == global_issuer
            || self
                .settings
                .issuers
                .iter()
                .any(|configured| configured.as_str() == issuer)
    }

    async fn opaque_bearer_credential(&self, token: &str) -> bool {
        if token.is_envelope() || token.is_refresh_envelope() || token.starts_with(V2_GCM_PREFIX) {
            return true;
        }
        let codec = SessionTokenCodec {
            cipher: StoredCipher {
                settings: CipherSettings {
                    signing_key: self.secret.clone(),
                    algorithm: Algorithm::Aes256Gcm,
                },
                nonces: SharedNonces(Arc::clone(&self.nonces)),
                diagnostics: TracingDiagnostics,
            },
            max_ui_session_budget: None,
            cli_expiration_hours: self.settings.cli_token_hours,
        };
        match codec.session_from_token(token) {
            Ok(Some(_)) => return true,
            Ok(None) => {}
            Err(error) => {
                tracing::debug!(error = %error, "OAuth bearer ownership could not be checked");
                return true;
            }
        }
        match self.store.identity_by_hash(&hash_token(token)).await {
            IdentityLookup::Found | IdentityLookup::Faulted => true,
            IdentityLookup::NotFound => false,
        }
    }
}
