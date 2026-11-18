use std::path::PathBuf;

use betterlite_config::sections::worker_options_reload::{
    LogConfig, LogLevelError, UvicornSettings,
};

use crate::worker_options::loop_type::{SysPlatform, resolve_loop_type};

const HEALTHCHECK_IGNORED: &str =
    "--timeout_worker_healthcheck is not supported by betterlite's server; ignoring the flag";
const JITTER_WITHOUT_LIMIT: &str =
    "--max_requests_before_restart_jitter has no effect without --max_requests_before_restart";

// an absent field leaves the config section's value in place, so an explicit flag is the only thing that wins
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UvicornOverrides {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub log_config: Option<PathBuf>,
    pub keepalive_timeout: Option<i64>,
    pub timeout_worker_healthcheck: Option<i64>,
    pub max_requests: Option<i64>,
    pub max_requests_jitter: Option<i64>,
}

// LiteLLM resolves LITELLM_LOG at import, before it reads the flag, so an unusable level ends the start here too
pub fn resolve_uvicorn_settings(
    base: &UvicornSettings,
    overrides: &UvicornOverrides,
    warn: &mut dyn FnMut(&str),
) -> Result<UvicornSettings, LogLevelError> {
    if let LogConfig::Invalid(error) = &base.log_config {
        return Err(error.clone());
    }
    let timeout_worker_healthcheck = overrides
        .timeout_worker_healthcheck
        .or(base.timeout_worker_healthcheck);
    if timeout_worker_healthcheck.is_some() {
        warn(HEALTHCHECK_IGNORED);
    }
    let limit_max_requests = overrides.max_requests.or(base.limit_max_requests);
    let requested_jitter = overrides
        .max_requests_jitter
        .or(base.limit_max_requests_jitter);
    Ok(UvicornSettings {
        host: overrides.host.clone().unwrap_or_else(|| base.host.clone()),
        port: overrides.port.unwrap_or(base.port),
        server_header: false,
        log_config: match overrides.log_config.clone() {
            Some(path) => LogConfig::File(path),
            None => base.log_config.clone(),
        },
        timeout_keep_alive: overrides.keepalive_timeout.or(base.timeout_keep_alive),
        timeout_worker_healthcheck,
        limit_max_requests,
        limit_max_requests_jitter: apply_max_requests_jitter(
            limit_max_requests,
            requested_jitter,
            warn,
        ),
        loop_type: resolve_loop_type(SysPlatform::current()),
    })
}

#[must_use]
pub fn apply_max_requests_jitter(
    limit_max_requests: Option<i64>,
    jitter: Option<i64>,
    warn: &mut dyn FnMut(&str),
) -> Option<i64> {
    let requested = jitter?;
    if limit_max_requests.is_none() {
        warn(JITTER_WITHOUT_LIMIT);
        return None;
    }
    Some(requested)
}
