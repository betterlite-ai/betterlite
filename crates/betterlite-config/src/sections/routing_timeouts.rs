use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number};
use crate::sections::Section;

mod tests;

// LiteLLM's package default: request_timeout is 6000 unless a config value or the REQUEST_TIMEOUT environment replaces it
const DEFAULT_TIMEOUT_SECONDS: f64 = 6000.0;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
    #[serde(default)]
    router_settings: RawRouter,
    #[serde(default)]
    litellm_settings: RawLiteLlm,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: RawParams,
}

#[derive(Debug, Default, Deserialize)]
struct RawParams {
    timeout: Option<RawNumber>,
    stream_timeout: Option<RawNumber>,
    request_timeout: Option<RawNumber>,
}

#[derive(Debug, Default, Deserialize)]
struct RawRouter {
    timeout: Option<RawNumber>,
    stream_timeout: Option<RawNumber>,
    #[serde(default)]
    default_litellm_params: RawDefaultParams,
}

#[derive(Debug, Default, Deserialize)]
struct RawDefaultParams {
    stream_timeout: Option<RawNumber>,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlm {
    request_timeout: Option<RawNumber>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlobalTimeout {
    pub seconds: f64,
    pub explicitly_set: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RouterTimeouts {
    pub timeout: Option<f64>,
    pub stream_timeout: Option<f64>,
    pub default_stream_timeout: Option<f64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DeploymentTimeouts {
    pub timeout: Option<f64>,
    pub stream_timeout: Option<f64>,
    pub request_timeout: Option<f64>,
}

// deployments stay index-aligned with model_list, which the router keeps in config order
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub global: GlobalTimeout,
    pub router: RouterTimeouts,
    pub deployments: Vec<DeploymentTimeouts>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            global: GlobalTimeout {
                seconds: DEFAULT_TIMEOUT_SECONDS,
                explicitly_set: false,
            },
            router: RouterTimeouts::default(),
            deployments: Vec::new(),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let configured = resolve_number(
            self.litellm_settings.request_timeout,
            "litellm_settings.request_timeout",
            lookup,
            parse_float,
        )?;
        let environment_text = lookup.lookup("REQUEST_TIMEOUT").unwrap_or_default();
        let environment = betterlite_env::number::<f64>("REQUEST_TIMEOUT", &environment_text)?;
        let (seconds, explicitly_set) = configured
            .map(|seconds| (seconds, true))
            .or_else(|| environment.map(|seconds| (seconds, true)))
            .unwrap_or((DEFAULT_TIMEOUT_SECONDS, false));
        let global = GlobalTimeout {
            seconds,
            explicitly_set,
        };
        Ok(Settings {
            global,
            router: router_settings(self.router_settings, lookup)?,
            deployments: deployment_settings(self.model_list, lookup)?,
        })
    }
}

fn router_settings<L>(raw: RawRouter, lookup: &L) -> Result<RouterTimeouts, ConfigError>
where
    L: EnvironmentLookup,
{
    let timeout = resolve_number(raw.timeout, "router_settings.timeout", lookup, parse_float)?;
    let stream_timeout = resolve_number(
        raw.stream_timeout,
        "router_settings.stream_timeout",
        lookup,
        parse_float,
    )?;
    let defaults = raw.default_litellm_params;
    let default_stream_timeout = resolve_number(
        defaults.stream_timeout,
        "router_settings.default_litellm_params.stream_timeout",
        lookup,
        parse_float,
    )?;
    Ok(RouterTimeouts {
        timeout,
        stream_timeout,
        default_stream_timeout,
    })
}

fn deployment_settings<L>(
    raw: Vec<RawDeployment>,
    lookup: &L,
) -> Result<Vec<DeploymentTimeouts>, ConfigError>
where
    L: EnvironmentLookup,
{
    let mut deployments = Vec::with_capacity(raw.len());
    for (index, deployment) in raw.into_iter().enumerate() {
        let params = deployment.litellm_params;
        let prefix = format!("model_list.{index}.litellm_params");
        let timeout = resolve_number(
            params.timeout,
            &format!("{prefix}.timeout"),
            lookup,
            parse_float,
        )?;
        let stream_timeout = resolve_number(
            params.stream_timeout,
            &format!("{prefix}.stream_timeout"),
            lookup,
            parse_float,
        )?;
        let request_timeout = resolve_number(
            params.request_timeout,
            &format!("{prefix}.request_timeout"),
            lookup,
            parse_float,
        )?;
        deployments.push(DeploymentTimeouts {
            timeout,
            stream_timeout,
            request_timeout,
        });
    }
    Ok(deployments)
}
