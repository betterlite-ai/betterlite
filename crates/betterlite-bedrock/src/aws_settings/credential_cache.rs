use std::array;
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use betterlite_context::local_ttl_cache::{
    CacheValue,
    credential::{Credential, CredentialHandle},
    dual::{DualLocalCache, Ttl},
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tokio::sync::Mutex;

use super::{
    AwsSessionTag,
    sources::{Credentials, ResolvedCredentials, SourceError},
};

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Parameter {
    Null,
    Boolean(bool),
    Text(SecretString),
    Tags(Vec<AwsSessionTag>),
}

#[derive(Serialize)]
#[serde(untagged)]
enum EncodedParameter<'value> {
    Null,
    Boolean(bool),
    Text(&'value str),
    Tags(&'value [AwsSessionTag]),
}

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("credential cache requires capacity 1000")]
    Capacity,
    #[error("credential cache unavailable")]
    Unavailable,
    #[error("invalid cached AWS credential handle")]
    Handle,
    #[error("credential parameters cannot be encoded")]
    Encoding(#[from] serde_json::Error),
    #[error(transparent)]
    Source(#[from] SourceError),
}

pub struct CredentialCache {
    storage: DualLocalCache,
    stripes: [Mutex<()>; 64],
}

pub fn cache_key(parameters: &BTreeMap<String, Parameter>) -> Result<String, CacheError> {
    let encoded: BTreeMap<_, _> = parameters
        .iter()
        .map(|(name, value)| {
            let field = match value {
                Parameter::Null => EncodedParameter::Null,
                Parameter::Boolean(boolean) => EncodedParameter::Boolean(*boolean),
                Parameter::Text(text) => EncodedParameter::Text(text.expose_secret()),
                Parameter::Tags(tags) => EncodedParameter::Tags(tags),
            };
            (name, field)
        })
        .collect();
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(&encoded)?)))
}

impl CredentialCache {
    pub fn new(storage: DualLocalCache) -> Result<Self, CacheError> {
        if storage
            .memory
            .lock()
            .map_err(|_poisoned| CacheError::Unavailable)?
            .as_ref()
            .capacity
            != 1000
        {
            return Err(CacheError::Capacity);
        }
        Ok(Self {
            storage,
            stripes: array::from_fn(|_| Mutex::new(())),
        })
    }

    pub async fn get_or_fetch<F, Fut>(
        &self,
        parameters: &BTreeMap<String, Parameter>,
        fetch: F,
    ) -> Result<Credentials, CacheError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<ResolvedCredentials, SourceError>>,
    {
        let key = cache_key(parameters)?;
        let stripe = key
            .bytes()
            .fold(0_usize, |sum, byte| sum.wrapping_add(usize::from(byte)))
            .checked_rem(self.stripes.len())
            .ok_or(CacheError::Unavailable)?;
        let lock = self.stripes.get(stripe).ok_or(CacheError::Unavailable)?;
        let _guard = lock.lock().await;
        if let Some(CacheValue::Credential(handle)) = self.storage.get(&key) {
            return Credentials::try_from(handle);
        }
        let resolved = fetch().await?;
        self.storage.set(
            &key,
            CacheValue::Credential(CredentialHandle::from(resolved.credentials.clone())),
            Ttl::Explicit(resolved.cache_ttl.map(|ttl| ttl.as_secs_f64())),
        );
        Ok(resolved.credentials)
    }
}

impl From<Credentials> for CredentialHandle {
    fn from(credentials: Credentials) -> Self {
        // These private fields store signing material, never outbound HTTP headers.
        let mut fields =
            BTreeMap::from([("aws_access_key_id".to_owned(), credentials.access_key_id)]);
        if let Some(token) = credentials.session_token {
            fields.insert("aws_session_token".to_owned(), token);
        }
        Self(Arc::new(Credential {
            token: credentials.secret_access_key,
            headers: fields,
        }))
    }
}

impl TryFrom<CredentialHandle> for Credentials {
    type Error = CacheError;

    fn try_from(handle: CredentialHandle) -> Result<Self, Self::Error> {
        let access_key_id = handle
            .0
            .headers
            .get("aws_access_key_id")
            .ok_or(CacheError::Handle)?
            .clone();
        Ok(Self {
            access_key_id,
            secret_access_key: handle.0.token.clone(),
            session_token: handle.0.headers.get("aws_session_token").cloned(),
        })
    }
}
