use std::str::FromStr;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, parse_integer, resolve_number, resolve_string};
use crate::sections::Section;

use self::policy::RawPolicy;

mod policy;
mod tests;

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
    #[serde(default)]
    model_info: RawModelInfo,
}

#[derive(Debug, Default, Deserialize)]
struct RawParams {
    cooldown_time: Option<RawNumber>,
}

#[derive(Debug, Default, Deserialize)]
struct RawModelInfo {
    #[serde(default)]
    allowed_fails_policy: Option<RawPolicy>,
    allowed_fails: Option<RawNumber>,
    cooldown_time: Option<RawNumber>,
}

#[derive(Debug, Default, Deserialize)]
struct RawRouter {
    allowed_fails: Option<RawNumber>,
    cooldown_time: Option<RawNumber>,
    #[serde(default)]
    disable_cooldowns: Option<RawFlag>,
    #[serde(default)]
    allowed_fails_policy: Option<RawPolicy>,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlm {
    allowed_fails: Option<RawNumber>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Boolean(bool),
    Text(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AllowedFailsPolicy {
    pub content_policy_violation: Option<u32>,
    pub bad_request: Option<u32>,
    pub authentication: Option<u32>,
    pub timeout: Option<u32>,
    pub rate_limit: Option<u32>,
    pub internal_server: Option<u32>,
    pub service_unavailable: Option<u32>,
    pub bad_gateway: Option<u32>,
    pub not_found: Option<u32>,
}

// index-aligned with model_list, which the deployment catalog keeps in config order
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeploymentCooldownSettings {
    pub allowed_fails_policy: Option<AllowedFailsPolicy>,
    pub allowed_fails: Option<u32>,
    pub model_info_cooldown_time: Option<f64>,
    pub litellm_params_cooldown_time: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    pub allowed_fails: u32,
    pub default_allowed_fails: u32,
    pub cooldown_time: f64,
    pub disable_cooldowns: bool,
    pub allowed_fails_policy: Option<AllowedFailsPolicy>,
    pub deployments: Vec<DeploymentCooldownSettings>,
    pub failure_ratio_threshold: f64,
    pub minimum_requests: u64,
    pub single_deployment_traffic: u64,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let configured_global = resolve_number(
            self.litellm_settings.allowed_fails,
            "litellm_settings.allowed_fails",
            lookup,
            parse_integer,
        )?;
        let environment_global = environment_number(lookup, "DEFAULT_ALLOWED_FAILS", 3_u32)?;
        let global_default = configured_global.unwrap_or(environment_global);
        let router = router_settings(self.router_settings, lookup)?;
        let deployments = deployment_settings(self.model_list, lookup)?;
        let (failure_ratio_threshold, minimum_requests, single_deployment_traffic) =
            thresholds(lookup)?;
        Ok(Settings {
            allowed_fails: router.allowed_fails.unwrap_or(global_default),
            default_allowed_fails: global_default,
            cooldown_time: router.cooldown_time.unwrap_or(environment_number(
                lookup,
                "DEFAULT_COOLDOWN_TIME_SECONDS",
                5.0_f64,
            )?),
            disable_cooldowns: router.disable_cooldowns,
            allowed_fails_policy: router.allowed_fails_policy,
            deployments,
            failure_ratio_threshold,
            minimum_requests,
            single_deployment_traffic,
        })
    }
}

struct RouterCooldown {
    allowed_fails: Option<u32>,
    cooldown_time: Option<f64>,
    disable_cooldowns: bool,
    allowed_fails_policy: Option<AllowedFailsPolicy>,
}

fn router_settings<L>(raw: RawRouter, lookup: &L) -> Result<RouterCooldown, ConfigError>
where
    L: EnvironmentLookup,
{
    let path = "router_settings.allowed_fails_policy";
    Ok(RouterCooldown {
        allowed_fails: resolve_number(
            raw.allowed_fails,
            "router_settings.allowed_fails",
            lookup,
            parse_integer,
        )?,
        cooldown_time: resolve_number(
            raw.cooldown_time,
            "router_settings.cooldown_time",
            lookup,
            parse_float,
        )?,
        disable_cooldowns: configured_flag(
            raw.disable_cooldowns,
            "router_settings.disable_cooldowns",
            lookup,
        )?,
        allowed_fails_policy: raw
            .allowed_fails_policy
            .map(|policy| policy.resolve(path, lookup))
            .transpose()?,
    })
}

fn deployment_settings<L>(
    raw: Vec<RawDeployment>,
    lookup: &L,
) -> Result<Vec<DeploymentCooldownSettings>, ConfigError>
where
    L: EnvironmentLookup,
{
    let mut deployments = Vec::with_capacity(raw.len());
    for (index, deployment) in raw.into_iter().enumerate() {
        deployments.push(one_deployment(deployment, index, lookup)?);
    }
    Ok(deployments)
}

fn one_deployment<L>(
    deployment: RawDeployment,
    index: usize,
    lookup: &L,
) -> Result<DeploymentCooldownSettings, ConfigError>
where
    L: EnvironmentLookup,
{
    let prefix = format!("model_list.{index}");
    let policy_path = format!("{prefix}.model_info.allowed_fails_policy");
    Ok(DeploymentCooldownSettings {
        allowed_fails_policy: deployment
            .model_info
            .allowed_fails_policy
            .map(|policy| policy.resolve(&policy_path, lookup))
            .transpose()?,
        allowed_fails: resolve_number(
            deployment.model_info.allowed_fails,
            &format!("{prefix}.model_info.allowed_fails"),
            lookup,
            parse_integer,
        )?,
        model_info_cooldown_time: resolve_number(
            deployment.model_info.cooldown_time,
            &format!("{prefix}.model_info.cooldown_time"),
            lookup,
            parse_float,
        )?,
        litellm_params_cooldown_time: resolve_number(
            deployment.litellm_params.cooldown_time,
            &format!("{prefix}.litellm_params.cooldown_time"),
            lookup,
            parse_float,
        )?,
    })
}

fn configured_flag<L>(value: Option<RawFlag>, path: &str, lookup: &L) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        None => Ok(false),
        Some(RawFlag::Boolean(flag)) => Ok(flag),
        Some(RawFlag::Text(text)) => {
            let name = text.strip_prefix("os.environ/").unwrap_or(path).to_owned();
            let resolved = resolve_string(text, path, lookup)?;
            Ok(betterlite_env::flag(&name, &resolved)?.unwrap_or(false))
        }
    }
}

fn thresholds<L>(lookup: &L) -> Result<(f64, u64, u64), ConfigError>
where
    L: EnvironmentLookup,
{
    let ratio = environment_number(lookup, "DEFAULT_FAILURE_THRESHOLD_PERCENT", 0.5_f64)?;
    let minimum = environment_number(lookup, "DEFAULT_FAILURE_THRESHOLD_MINIMUM_REQUESTS", 5_u64)?;
    let single = environment_number(
        lookup,
        "SINGLE_DEPLOYMENT_TRAFFIC_FAILURE_THRESHOLD",
        1000_u64,
    )?;
    Ok((ratio, minimum, single))
}

fn environment_number<L, T>(lookup: &L, name: &str, default: T) -> Result<T, ConfigError>
where
    L: EnvironmentLookup,
    T: FromStr,
{
    let text = lookup.lookup(name).unwrap_or_default();
    let configured = betterlite_env::number::<T>(name, &text)?;
    Ok(configured.unwrap_or(default))
}
