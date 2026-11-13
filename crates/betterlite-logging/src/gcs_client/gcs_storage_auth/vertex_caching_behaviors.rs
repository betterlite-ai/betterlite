use std::sync::{Arc, PoisonError};

use secrecy::SecretString;

use crate::gcs_mock::auth::{EnsureAccessToken, GcsMockAuth, VertexAuth, VertexAuthProvider};
use crate::gcs_mock::client::GcsHttpClient;

use super::caching_key_behaviors::{CredentialKey, vertex_instance_key};
use super::error::GcsClientError;
use super::{GcsBucketBase, VertexInstance};

pub async fn get_or_create_vertex_instance<Auth, Http>(
    base: &GcsBucketBase<Auth, Http>,
    credentials: Option<&SecretString>,
) -> Result<VertexInstance<Auth>, GcsClientError<Auth, Http>>
where
    Auth: VertexAuth + Clone,
    Http: GcsHttpClient,
{
    let key = CredentialKey(vertex_instance_key(credentials));
    let cached = base
        .vertex_instances
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&key)
        .cloned();
    if let Some(instance) = cached {
        return Ok(instance);
    }
    let instance: VertexInstance<Auth> = Arc::new(GcsMockAuth {
        real: base.auth.real.clone(),
        installed: base.auth.installed,
    });
    instance
        .ensure_access_token_async(EnsureAccessToken {
            credentials: credentials.cloned(),
            project_id: None,
            custom_llm_provider: VertexAuthProvider::VertexAi,
        })
        .await
        .map_err(GcsClientError::Token)?;
    base.vertex_instances
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(key, Arc::clone(&instance));
    Ok(instance)
}

mod tests;
