use async_trait::async_trait;

use crate::auth_cache::backend::AuthCacheTier;
use crate::auth_cache::{AuthCache, AuthCacheError, WriteOptions, WriteTtl};

use super::idp_metadata::IdpSettings;

pub const IDP_SETTINGS_PREFIX: &str = "saml_idp_settings";
pub const AUTHN_REQUEST_PREFIX: &str = "saml_authn_request";
pub const IDP_TTL: f64 = 3600.0;
pub const AUTHN_TTL: f64 = 600.0;
pub const AUTHN_REQUEST_MARKER: &str = "1";

// LiteLLM reads and writes the user api key cache directly and a backend failure never fails the request
#[async_trait]
pub trait SamlCache: Send + Sync {
    async fn get_idp_settings(
        &self,
        cache_key: &str,
    ) -> Result<Option<IdpSettings>, AuthCacheError>;
    async fn put_idp_settings(
        &self,
        cache_key: &str,
        settings: &IdpSettings,
        ttl_seconds: f64,
    ) -> Result<(), AuthCacheError>;
    async fn put_authn_request(
        &self,
        cache_key: &str,
        ttl_seconds: f64,
    ) -> Result<(), AuthCacheError>;
}

#[async_trait]
impl<T> SamlCache for AuthCache<T>
where
    T: AuthCacheTier + 'static,
{
    async fn get_idp_settings(
        &self,
        cache_key: &str,
    ) -> Result<Option<IdpSettings>, AuthCacheError> {
        self.async_get_typed(cache_key, false)
            .await
            .map_err(AuthCacheError::from)
    }

    async fn put_idp_settings(
        &self,
        cache_key: &str,
        settings: &IdpSettings,
        ttl_seconds: f64,
    ) -> Result<(), AuthCacheError> {
        let ttl = WriteTtl {
            seconds: ttl_seconds,
        };
        let options = WriteOptions {
            ttl: Some(ttl),
            local_only: false,
            refresh_ttl: false,
        };
        self.async_set(Some(cache_key), settings, options).await
    }

    async fn put_authn_request(
        &self,
        cache_key: &str,
        ttl_seconds: f64,
    ) -> Result<(), AuthCacheError> {
        self.async_set(
            Some(cache_key),
            &AUTHN_REQUEST_MARKER,
            WriteOptions {
                ttl: Some(WriteTtl {
                    seconds: ttl_seconds,
                }),
                local_only: false,
                refresh_ttl: false,
            },
        )
        .await
    }
}
