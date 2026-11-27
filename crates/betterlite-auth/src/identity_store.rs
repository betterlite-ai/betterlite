pub mod error;
pub mod ports;
pub mod principal;
pub mod rows;

use std::fmt;

use opentelemetry::trace::SpanContext;
use secrecy::ExposeSecret as _;

use self::error::IdentityStoreError;
use self::ports::Dependencies;
use self::principal::{Projection, principal_from_key};
use self::rows::auth_from_view;
use crate::auth_cache::copy::copy_for_cache;
use crate::auth_identity::records::{CredentialRef, NetworkContext, Principal};
use crate::auth_identity::roles::AuthMethod;
use crate::auth_key_lookup::error::KeyLookupError;
use crate::resolved_key::auth::UserAPIKeyAuth;

mod tests;

// store.py's resolver over the ports B733, B734, B735 and B738 own; each store belongs to one request's span and cache mode.
#[derive(Clone)]
pub struct IdentityStore {
    dependencies: Dependencies,
    parent_otel_span: Option<SpanContext>,
    check_cache_only: bool,
}

impl fmt::Debug for IdentityStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IdentityStore")
            .field("check_cache_only", &self.check_cache_only)
            .finish_non_exhaustive()
    }
}

impl IdentityStore {
    #[must_use]
    pub const fn new(
        dependencies: Dependencies,
        parent_otel_span: Option<SpanContext>,
        check_cache_only: bool,
    ) -> Self {
        Self {
            dependencies,
            parent_otel_span,
            check_cache_only,
        }
    }

    pub async fn resolve(&self, hashed_token: &str) -> Result<Principal, IdentityStoreError> {
        self.resolve_as(hashed_token, AuthMethod::ApiKey, None)
            .await
    }

    pub async fn resolve_as(
        &self,
        hashed_token: &str,
        auth_method: AuthMethod,
        network: Option<NetworkContext>,
    ) -> Result<Principal, IdentityStoreError> {
        let key = self.resolve_key(hashed_token).await?;
        let token = key
            .view
            .token
            .token
            .as_ref()
            .map(|value| value.expose_secret().to_owned());
        Ok(principal_from_key(
            &key,
            Projection {
                auth_method,
                issuer: None,
                subject_fallback: token.as_deref(),
                scopes: &[],
                credential_ref: Some(CredentialRef {
                    key_id: None,
                    token_id: token,
                }),
                network,
            },
        ))
    }

    pub fn key_from_principal(principal: &Principal) -> Result<UserAPIKeyAuth, IdentityStoreError> {
        principal
            .source_key
            .0
            .clone()
            .ok_or(IdentityStoreError::PrincipalMissingSourceKey)
    }

    // _resolve_key: the database presence leads even a warm cache, then the cache, then the reconnecting reader.
    async fn resolve_key(&self, hashed_token: &str) -> Result<UserAPIKeyAuth, IdentityStoreError> {
        let Some(reader) = self.dependencies.reader.as_ref() else {
            return Err(IdentityStoreError::KeyLookup(KeyLookupError::NoDatabase));
        };
        if let Some(cached) = self
            .dependencies
            .cache
            .read_key_object(hashed_token)
            .await?
        {
            return Ok(copy_for_cache(&cached));
        }
        if self.check_cache_only {
            return Err(IdentityStoreError::KeyLookup(KeyLookupError::NotInCache {
                key: hashed_token.to_owned(),
            }));
        }
        let Some(view) = reader
            .fetch(hashed_token, false, self.parent_otel_span.clone())
            .await?
        else {
            return Err(IdentityStoreError::KeyLookup(KeyLookupError::MissingKey {
                key: hashed_token.to_owned(),
            }));
        };
        let mut key = auth_from_view(&view)?;
        self.load_permission(&mut key).await;
        self.dependencies
            .writer
            .write_key(hashed_token, &mut key)
            .await
            .map_err(|error| KeyLookupError::CacheWrite(Box::new(error)))?;
        Ok(key)
    }

    // A failed permission read is logged and ignored; authentication continues without the relation.
    async fn load_permission(&self, key: &mut UserAPIKeyAuth) {
        let Some(object_permission_id) = key
            .view
            .token
            .object_permission_id
            .as_deref()
            .filter(|id| !id.is_empty())
        else {
            return;
        };
        if key.view.token.object_permission.is_some() {
            return;
        }
        match self
            .dependencies
            .permissions
            .permission(object_permission_id, false)
            .await
        {
            Ok(permission) => key.view.token.object_permission = permission,
            Err(error) => tracing::debug!(
                "Failed to load object_permission for key with object_permission_id={object_permission_id}: {error}"
            ),
        }
    }
}
