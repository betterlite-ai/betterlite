use async_trait::async_trait;
use betterlite_models::decisions::Json;

use crate::jwt_keys::errors::KeyError;
use crate::jwt_keys::resolver::JwksResolver;

// The key lookups auth_jwt makes, so a test or helper supplies a key without a network or a cache.
#[async_trait]
pub trait PublicKeySource: Send + Sync {
    async fn resolve_public_key(&self, kid: Option<&str>) -> Result<Json, KeyError>;
    async fn resolve_keys_for_url(
        &self,
        jwks_url: &str,
        kid: Option<&str>,
    ) -> Result<Json, KeyError>;
}

#[async_trait]
impl PublicKeySource for JwksResolver {
    async fn resolve_public_key(&self, kid: Option<&str>) -> Result<Json, KeyError> {
        Self::get_public_key(self, kid).await
    }

    async fn resolve_keys_for_url(
        &self,
        jwks_url: &str,
        kid: Option<&str>,
    ) -> Result<Json, KeyError> {
        Self::keys_for_url(self, jwks_url, kid).await
    }
}
