use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_models::sandbox::{ContainerHandle, SandboxOutputError};
use http::header::InvalidHeaderName;
use secrecy::SecretString;
use serde_json::value::RawValue;
use thiserror::Error;

use super::helpers::OpenSandboxError;

mod delete;
mod operations;
mod tests;

#[derive(Debug, Error)]
pub enum LifecycleError {
    #[error("the execd endpoint returned a header name a request cannot carry")]
    InvalidHeaderName {
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the sandbox response body could not be read")]
    Body {
        #[from]
        source: SandboxOutputError<OpenSandboxError>,
    },
    #[error(transparent)]
    Sandbox(#[from] OpenSandboxError),
}

#[derive(Debug, Clone)]
pub struct SandboxHandle {
    pub container: ContainerHandle,
    pub execd_endpoint: String,
    pub execd_headers: BTreeMap<String, SecretString>,
    pub use_server_proxy: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum ContainerRef<'request> {
    Cached(&'request SandboxHandle),
    Handle(&'request ContainerHandle),
    Id(&'request str),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SandboxOptions<'request> {
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
    pub use_server_proxy: bool,
    pub ready_timeout: Option<Duration>,
    pub poll_interval: Option<Duration>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CreateSandboxRequest<'request> {
    pub template: Option<&'request str>,
    pub timeout: Option<i64>,
    pub allow_internet_access: Option<bool>,
    pub metadata: Option<&'request BTreeMap<String, String>>,
    pub env_vars: Option<&'request BTreeMap<String, String>>,
    pub resource_limits: Option<&'request BTreeMap<String, String>>,
    pub resource_requests: Option<&'request BTreeMap<String, String>>,
    pub entrypoint: Option<&'request [String]>,
    pub network_policy: Option<&'request RawValue>,
    pub secure_access: bool,
    pub options: SandboxOptions<'request>,
}

#[derive(Debug, Clone, Copy)]
pub struct RunCode<'request> {
    pub container: ContainerRef<'request>,
    pub code: &'request str,
    pub language: Option<&'request str>,
    pub options: SandboxOptions<'request>,
}

#[derive(Debug, Clone, Copy)]
pub struct DeleteSandbox<'request> {
    pub container: ContainerRef<'request>,
    pub options: SandboxOptions<'request>,
}
