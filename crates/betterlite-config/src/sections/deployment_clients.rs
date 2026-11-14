use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_integer, resolve_number};
use crate::sections::Section;

mod tests;

// RouterConfig.client_ttl and the Router constructor's default
const DEFAULT_CLIENT_TTL_SECONDS: u64 = 3600;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
    #[serde(default)]
    router_settings: RawRouter,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: RawParams,
}

#[derive(Debug, Default, Deserialize)]
struct RawParams {
    max_parallel_requests: Option<RawNumber>,
    rpm: Option<RawNumber>,
    tpm: Option<RawNumber>,
}

#[derive(Debug, Default, Deserialize)]
struct RawRouter {
    default_max_parallel_requests: Option<RawNumber>,
    client_ttl: Option<RawNumber>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DeploymentLimits {
    pub max_parallel_requests: Option<i64>,
    pub rpm: Option<i64>,
    pub tpm: Option<i64>,
}

// deployments stay index-aligned with model_list, like the router's config order
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Settings {
    pub client_ttl_seconds: u64,
    pub default_max_parallel_requests: Option<i64>,
    pub deployments: Box<[DeploymentLimits]>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            client_ttl_seconds: DEFAULT_CLIENT_TTL_SECONDS,
            default_max_parallel_requests: None,
            deployments: Box::default(),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let client_ttl_seconds = resolve_number(
            self.router_settings.client_ttl,
            "router_settings.client_ttl",
            lookup,
            parse_integer,
        )?
        .map_or(DEFAULT_CLIENT_TTL_SECONDS, u64::from);
        let default_max_parallel_requests = resolve_number(
            self.router_settings.default_max_parallel_requests,
            "router_settings.default_max_parallel_requests",
            lookup,
            parse_integer,
        )?
        .map(i64::from);
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            deployments.push(deployment_limits(index, deployment.litellm_params, lookup)?);
        }
        Ok(Settings {
            client_ttl_seconds,
            default_max_parallel_requests,
            deployments: deployments.into_boxed_slice(),
        })
    }
}

fn deployment_limits<L>(
    index: usize,
    params: RawParams,
    lookup: &L,
) -> Result<DeploymentLimits, ConfigError>
where
    L: EnvironmentLookup,
{
    let path = |field: &str| format!("model_list.{index}.litellm_params.{field}");
    Ok(DeploymentLimits {
        max_parallel_requests: resolve_number(
            params.max_parallel_requests,
            &path("max_parallel_requests"),
            lookup,
            parse_integer,
        )?
        .map(i64::from),
        rpm: resolve_number(params.rpm, &path("rpm"), lookup, parse_integer)?.map(i64::from),
        tpm: resolve_number(params.tpm, &path("tpm"), lookup, parse_integer)?.map(i64::from),
    })
}
