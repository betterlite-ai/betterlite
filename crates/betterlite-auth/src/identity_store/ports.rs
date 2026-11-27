use std::sync::Arc;

use async_trait::async_trait;

use crate::auth_cache::AuthCache;
use crate::auth_cache::backend::AuthCacheTier;
use crate::auth_key_lookup::cache::KeyCacheWriter;
use crate::auth_key_lookup::permission::KeyPermissionLookup;
use crate::auth_key_lookup::reader::KeyReader;
use crate::resolved_key::auth::{AuthError, UserAPIKeyAuth};

use super::error::IdentityStoreError;

// The key-object cache read B733 owns; unlike B738's port, a payload that no longer validates is an error here.
#[async_trait]
pub trait KeyObjectCache: Send + Sync {
    async fn read_key_object(
        &self,
        hashed_token: &str,
    ) -> Result<Option<UserAPIKeyAuth>, AuthError>;
}

#[async_trait]
impl<Tier> KeyObjectCache for AuthCache<Tier>
where
    Tier: AuthCacheTier,
{
    async fn read_key_object(
        &self,
        hashed_token: &str,
    ) -> Result<Option<UserAPIKeyAuth>, AuthError> {
        self.async_get_auth(hashed_token, false).await
    }
}

#[derive(Clone)]
pub struct Dependencies {
    // a database-free deployment keeps the runtime NoDatabaseConnection error and a None reader
    pub reader: Option<Arc<KeyReader>>,
    pub cache: Arc<dyn KeyObjectCache>,
    pub permissions: Arc<dyn KeyPermissionLookup>,
    pub writer: Arc<dyn KeyCacheWriter>,
}

#[derive(Default)]
pub struct Ports {
    pub reader: Option<Arc<KeyReader>>,
    pub cache: Option<Arc<dyn KeyObjectCache>>,
    pub permissions: Option<Arc<dyn KeyPermissionLookup>>,
    pub writer: Option<Arc<dyn KeyCacheWriter>>,
}

impl Ports {
    pub fn require(self) -> Result<Dependencies, IdentityStoreError> {
        let Self {
            reader,
            cache,
            permissions,
            writer,
        } = self;
        let mut missing = Vec::new();
        if cache.is_none() {
            missing.push("cache");
        }
        if permissions.is_none() {
            missing.push("permissions");
        }
        if writer.is_none() {
            missing.push("writer");
        }
        if !missing.is_empty() {
            return Err(IdentityStoreError::MissingPorts {
                ports: missing.join(", "),
            });
        }
        let (Some(cache_port), Some(permissions_port), Some(writer_port)) =
            (cache, permissions, writer)
        else {
            return Err(IdentityStoreError::MissingPorts {
                ports: missing.join(", "),
            });
        };
        Ok(Dependencies {
            reader,
            cache: cache_port,
            permissions: permissions_port,
            writer: writer_port,
        })
    }
}
