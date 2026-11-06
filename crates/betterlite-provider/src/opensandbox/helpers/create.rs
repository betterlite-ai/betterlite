use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::value::RawValue;

use super::constants::{
    DEFAULT_CPU_LIMIT, DEFAULT_ENTRYPOINT, DEFAULT_MEMORY_LIMIT, DEFAULT_TEMPLATE, DEFAULT_TIMEOUT,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct CreateSandbox<'request> {
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
}

#[derive(Debug, Serialize)]
pub struct CreateBody {
    pub image: ImageSpec,
    pub entrypoint: Vec<String>,
    pub timeout: i64,
    #[serde(rename = "resourceLimits")]
    pub resource_limits: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, String>>,
    #[serde(rename = "resourceRequests", skip_serializing_if = "Option::is_none")]
    pub resource_requests: Option<BTreeMap<String, String>>,
    #[serde(rename = "networkPolicy", skip_serializing_if = "Option::is_none")]
    pub network_policy: Option<NetworkPolicy>,
    #[serde(rename = "secureAccess", skip_serializing_if = "Option::is_none")]
    pub secure_access: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct ImageSpec {
    pub uri: String,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum NetworkPolicy {
    Given(Box<RawValue>),
    Deny(DenyPolicy),
}

#[derive(Debug, Serialize)]
pub struct DenyPolicy {
    #[serde(rename = "defaultAction")]
    pub default_action: String,
    pub egress: Vec<String>,
}

#[must_use]
pub fn create_body(request: &CreateSandbox<'_>) -> CreateBody {
    CreateBody {
        image: ImageSpec {
            uri: request.template.unwrap_or(DEFAULT_TEMPLATE).to_owned(),
        },
        entrypoint: request.entrypoint.map_or_else(
            || DEFAULT_ENTRYPOINT.map(str::to_owned).to_vec(),
            <[String]>::to_vec,
        ),
        timeout: request.timeout.unwrap_or(DEFAULT_TIMEOUT),
        resource_limits: request
            .resource_limits
            .map_or_else(default_resource_limits, Clone::clone),
        metadata: request.metadata.cloned(),
        env: request.env_vars.cloned(),
        resource_requests: request.resource_requests.cloned(),
        network_policy: match (request.network_policy, request.allow_internet_access) {
            (Some(policy), _ignored) => Some(NetworkPolicy::Given(policy.to_owned())),
            (None, Some(true)) => None,
            (None, _ignored) => Some(NetworkPolicy::Deny(DenyPolicy {
                default_action: "deny".to_owned(),
                egress: Vec::new(),
            })),
        },
        secure_access: request.secure_access.then_some(true),
    }
}

#[must_use]
pub fn default_resource_limits() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("cpu".to_owned(), DEFAULT_CPU_LIMIT.to_owned()),
        ("memory".to_owned(), DEFAULT_MEMORY_LIMIT.to_owned()),
    ])
}
