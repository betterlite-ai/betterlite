use std::str::FromStr;

use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, parse_integer, resolve_number};
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    router_settings: RawRouter,
}

#[derive(Debug, Default, Deserialize)]
struct RawRouter {
    allowed_fails: Option<RawNumber>,
    cooldown_time: Option<RawNumber>,
    #[serde(default)]
    disable_cooldowns: Option<bool>,
    #[serde(default)]
    allowed_fails_policy: Option<AllowedFailsPolicy>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct AllowedFailsPolicy {
    #[serde(default, rename = "ContentPolicyViolationErrorAllowedFails")]
    pub content_policy_violation: Option<u32>,
    #[serde(default, rename = "BadRequestErrorAllowedFails")]
    pub bad_request: Option<u32>,
    #[serde(default, rename = "AuthenticationErrorAllowedFails")]
    pub authentication: Option<u32>,
    #[serde(default, rename = "TimeoutErrorAllowedFails")]
    pub timeout: Option<u32>,
    #[serde(default, rename = "RateLimitErrorAllowedFails")]
    pub rate_limit: Option<u32>,
    #[serde(default, rename = "InternalServerErrorAllowedFails")]
    pub internal_server: Option<u32>,
    #[serde(default, rename = "ServiceUnavailableErrorAllowedFails")]
    pub service_unavailable: Option<u32>,
    #[serde(default, rename = "BadGatewayErrorAllowedFails")]
    pub bad_gateway: Option<u32>,
    #[serde(default, rename = "NotFoundErrorAllowedFails")]
    pub not_found: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    pub allowed_fails: u32,
    pub default_allowed_fails: u32,
    pub cooldown_time: f64,
    pub disable_cooldowns: bool,
    pub allowed_fails_policy: Option<AllowedFailsPolicy>,
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
        let default_allowed_fails = environment_number(lookup, "DEFAULT_ALLOWED_FAILS", 3_u32)?;
        let configured_fails = resolve_number(
            self.router_settings.allowed_fails,
            "router_settings.allowed_fails",
            lookup,
            parse_integer,
        )?;
        let default_cooldown =
            environment_number(lookup, "DEFAULT_COOLDOWN_TIME_SECONDS", 5.0_f64)?;
        let configured_cooldown = resolve_number(
            self.router_settings.cooldown_time,
            "router_settings.cooldown_time",
            lookup,
            parse_float,
        )?;
        let (failure_ratio_threshold, minimum_requests, single_deployment_traffic) =
            thresholds(lookup)?;
        Ok(Settings {
            allowed_fails: configured_fails.unwrap_or(default_allowed_fails),
            default_allowed_fails,
            cooldown_time: configured_cooldown.unwrap_or(default_cooldown),
            disable_cooldowns: self.router_settings.disable_cooldowns.unwrap_or(false),
            allowed_fails_policy: self.router_settings.allowed_fails_policy,
            failure_ratio_threshold,
            minimum_requests,
            single_deployment_traffic,
        })
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
