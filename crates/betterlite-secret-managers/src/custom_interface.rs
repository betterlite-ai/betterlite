use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use secrecy::SecretString;

pub type CustomSecretFuture<'borrowed, T> =
    Pin<Box<dyn Future<Output = Result<T, CustomSecretError>> + Send + 'borrowed>>;

pub type CustomParams = BTreeMap<String, CustomValue>;
pub type CustomResponse = BTreeMap<String, CustomValue>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomValue {
    Null,
    Bool(bool),
    Integer(i64),
    Number(String),
    String(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

#[derive(Debug)]
pub struct CustomSecretWrite<'borrowed> {
    pub secret_name: &'borrowed str,
    pub secret_value: &'borrowed SecretString,
    pub description: Option<&'borrowed str>,
    pub optional_params: Option<&'borrowed CustomParams>,
    pub tags: Option<&'borrowed CustomValue>,
}

#[derive(Debug, thiserror::Error)]
pub enum CustomSecretError {
    #[error("operation is not implemented for {manager}; override {method}() to add support")]
    NotImplemented {
        manager: String,
        method: &'static str,
    },
    #[error("custom secret manager operation failed")]
    Operation(#[source] CustomOperationFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomSecretOperation {
    AsyncRead,
    SyncRead,
    AsyncWrite,
    AsyncDelete,
}

#[derive(Debug, thiserror::Error)]
#[error("{operation:?} raised {exception_type}: {message}")]
pub struct CustomOperationFailure {
    pub operation: CustomSecretOperation,
    pub exception_type: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomSecretManagerName(pub String);

impl From<Option<&str>> for CustomSecretManagerName {
    fn from(manager_name: Option<&str>) -> Self {
        manager_name.map_or_else(
            || Self("custom_secret_manager".to_owned()),
            |provided_name| Self(provided_name.to_owned()),
        )
    }
}

pub trait CustomSecretManager: Send + Sync {
    fn secret_manager_name(&self) -> &str;

    fn async_read_secret<'borrowed>(
        &'borrowed self,
        secret_name: &'borrowed str,
        optional_params: Option<&'borrowed CustomParams>,
    ) -> CustomSecretFuture<'borrowed, Option<SecretString>>;

    fn sync_read_secret(
        &self,
        secret_name: &str,
        optional_params: Option<&CustomParams>,
    ) -> Result<Option<SecretString>, CustomSecretError>;

    fn async_write_secret<'borrowed>(
        &'borrowed self,
        _write: CustomSecretWrite<'borrowed>,
    ) -> CustomSecretFuture<'borrowed, CustomResponse> {
        let manager = self.secret_manager_name().to_owned();
        Box::pin(async move {
            Err(CustomSecretError::NotImplemented {
                manager,
                method: "async_write_secret",
            })
        })
    }

    fn async_delete_secret<'borrowed>(
        &'borrowed self,
        _secret_name: &'borrowed str,
        _recovery_window_in_days: Option<u32>,
        _optional_params: Option<&'borrowed CustomParams>,
    ) -> CustomSecretFuture<'borrowed, CustomResponse> {
        let manager = self.secret_manager_name().to_owned();
        Box::pin(async move {
            Err(CustomSecretError::NotImplemented {
                manager,
                method: "async_delete_secret",
            })
        })
    }
}

#[path = "custom_interface/tests.rs"]
mod tests;
