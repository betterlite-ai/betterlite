use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_models::sandbox::ContainerHandle;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::value::RawValue;

use crate::opensandbox::lifecycle::SandboxHandle;

use super::logging::SandboxLogging;

pub struct SandboxCall<'call> {
    pub provider: &'call str,
    pub api_key: Option<&'call SecretString>,
    pub kwargs: &'call IndexMap<String, Json>,
    pub logging: Option<&'call mut dyn SandboxLogging>,
}

pub struct CreateSandbox<'call> {
    pub call: SandboxCall<'call>,
    pub api_base: Option<&'call str>,
    pub template: Option<&'call str>,
    pub timeout: Option<i64>,
    pub allow_internet_access: Option<bool>,
    pub metadata: Option<&'call BTreeMap<String, String>>,
    pub env_vars: Option<&'call BTreeMap<String, String>>,
    pub resource_limits: Option<&'call BTreeMap<String, String>>,
    pub resource_requests: Option<&'call BTreeMap<String, String>>,
    pub entrypoint: Option<&'call [String]>,
    pub network_policy: Option<&'call RawValue>,
    pub secure_access: bool,
    pub use_server_proxy: bool,
    pub ready_timeout: Option<Duration>,
    pub poll_interval: Option<Duration>,
}

pub struct RunCode<'call> {
    pub call: SandboxCall<'call>,
    pub container: SandboxContainerRef<'call>,
    pub code: &'call str,
    pub api_base: Option<&'call str>,
    pub env_vars: Option<&'call BTreeMap<String, String>>,
    pub language: Option<&'call str>,
    pub use_server_proxy: bool,
    pub ready_timeout: Option<Duration>,
    pub poll_interval: Option<Duration>,
}

pub struct DeleteSandbox<'call> {
    pub call: SandboxCall<'call>,
    pub container: SandboxContainerRef<'call>,
    pub api_base: Option<&'call str>,
}

#[derive(Debug, Clone)]
pub enum SandboxContainer {
    E2b(ContainerHandle),
    OpenSandbox(SandboxHandle),
}

#[derive(Debug, Clone, Copy)]
pub enum SandboxContainerRef<'call> {
    Container(&'call SandboxContainer),
    Id(&'call str),
}
