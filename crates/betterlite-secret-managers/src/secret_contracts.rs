use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type SecretParams = BTreeMap<String, SecretValue>;
pub type SecretResponse = BTreeMap<String, SecretValue>;
pub type SecretFuture<'borrowed, T> =
    Pin<Box<dyn Future<Output = Result<T, SecretContractError>> + Send + 'borrowed>>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SecretValue {
    Null,
    Boolean(bool),
    Integer(i64),
    Unsigned(u64),
    Float(f64),
    String(String),
    List(Vec<Self>),
    Mapping(BTreeMap<String, Self>),
}

#[derive(Debug, Clone, Copy)]
pub struct SecretOperationOptions<'borrowed> {
    pub optional_params: Option<&'borrowed SecretParams>,
    pub timeout: Option<Duration>,
}

#[derive(Debug, Clone, Copy)]
pub struct SecretWrite<'borrowed> {
    pub secret_name: &'borrowed str,
    pub secret_value: &'borrowed str,
    pub description: Option<&'borrowed str>,
    pub options: SecretOperationOptions<'borrowed>,
    pub tags: Option<&'borrowed SecretValue>,
}

#[derive(Debug, Clone, Copy)]
pub struct SecretRotation<'borrowed> {
    pub current_secret_name: &'borrowed str,
    pub new_secret_name: &'borrowed str,
    pub new_secret_value: &'borrowed str,
    pub options: SecretOperationOptions<'borrowed>,
}

#[derive(Debug, Error)]
pub enum SecretContractError {
    #[error("Current secret {name} not found")]
    CurrentSecretNotFound { name: String },
    #[error("Failed to verify new secret {name}")]
    NewSecretNotVerified { name: String },
    #[error("secret operation failed: {message}")]
    Operation { message: String },
}

pub trait BaseSecretManager: Sync {
    fn async_read_secret<'borrowed>(
        &'borrowed self,
        secret_name: &'borrowed str,
        options: SecretOperationOptions<'borrowed>,
    ) -> SecretFuture<'borrowed, Option<String>>;

    fn sync_read_secret(
        &self,
        secret_name: &str,
        options: SecretOperationOptions<'_>,
    ) -> Result<Option<String>, SecretContractError>;

    fn async_write_secret<'borrowed>(
        &'borrowed self,
        write: SecretWrite<'borrowed>,
    ) -> SecretFuture<'borrowed, SecretResponse>;

    fn async_delete_secret<'borrowed>(
        &'borrowed self,
        secret_name: &'borrowed str,
        recovery_window_in_days: Option<u32>,
        options: SecretOperationOptions<'borrowed>,
    ) -> SecretFuture<'borrowed, SecretResponse>;

    fn async_rotate_secret<'borrowed>(
        &'borrowed self,
        rotation: SecretRotation<'borrowed>,
    ) -> SecretFuture<'borrowed, SecretResponse> {
        Box::pin(async move {
            let old_secret = self
                .async_read_secret(rotation.current_secret_name, rotation.options)
                .await?;
            if old_secret.is_none() {
                return Err(SecretContractError::CurrentSecretNotFound {
                    name: rotation.current_secret_name.to_owned(),
                });
            }
            let description = format!("Rotated from {}", rotation.current_secret_name);
            let create_response = self
                .async_write_secret(SecretWrite {
                    secret_name: rotation.new_secret_name,
                    secret_value: rotation.new_secret_value,
                    description: Some(&description),
                    options: rotation.options,
                    tags: None,
                })
                .await?;
            let new_secret = self
                .async_read_secret(rotation.new_secret_name, rotation.options)
                .await?;
            if new_secret.is_none() {
                return Err(SecretContractError::NewSecretNotVerified {
                    name: rotation.new_secret_name.to_owned(),
                });
            }
            self.async_delete_secret(rotation.current_secret_name, Some(7), rotation.options)
                .await?;
            Ok(create_response)
        })
    }
}

#[path = "secret_contracts/tests.rs"]
mod tests;
