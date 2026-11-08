use std::time::Instant;

use betterlite_config::sections::routing_timeouts as config;
use betterlite_router::routing_timeouts::error::TimeoutError;
use betterlite_router::routing_timeouts::global::GlobalRequestTimeout;
use betterlite_router::routing_timeouts::plan::{
    DeploymentTimeouts, RequestTimeouts, RouterTimeoutSources, RouterTimeouts, TimeoutPlan,
    resolve_router_timeouts,
};
use betterlite_router::routing_timeouts::seconds::Seconds;
use thiserror::Error;

mod tests;

#[derive(Debug, Error)]
pub enum RoutingTimeoutError {
    #[error(transparent)]
    Timeout {
        #[from]
        source: TimeoutError,
    },
    #[error("deployment {index} is not in the configured model list")]
    UnknownDeployment { index: usize },
}

#[derive(Debug)]
pub struct TimeoutSettings {
    pub global: GlobalRequestTimeout,
    pub router: RouterTimeouts,
    pub deployments: Vec<DeploymentTimeouts>,
}

pub fn resolve(section: &config::Settings) -> Result<TimeoutSettings, RoutingTimeoutError> {
    let global = GlobalRequestTimeout {
        seconds: Seconds::new("litellm_settings.request_timeout", section.global.seconds)?,
        explicitly_set: section.global.explicitly_set,
    };
    let sources = RouterTimeoutSources {
        timeout: optional("router_settings.timeout", section.router.timeout)?,
        stream_timeout: optional(
            "router_settings.stream_timeout",
            section.router.stream_timeout,
        )?,
        default_stream_timeout: optional(
            "router_settings.default_litellm_params.stream_timeout",
            section.router.default_stream_timeout,
        )?,
    };
    let router = resolve_router_timeouts(&sources, global);
    let mut deployments = Vec::with_capacity(section.deployments.len());
    for (index, deployment) in section.deployments.iter().enumerate() {
        let prefix = format!("model_list.{index}.litellm_params");
        deployments.push(DeploymentTimeouts {
            timeout: optional(&format!("{prefix}.timeout"), deployment.timeout)?,
            stream_timeout: optional(
                &format!("{prefix}.stream_timeout"),
                deployment.stream_timeout,
            )?,
            request_timeout: optional(
                &format!("{prefix}.request_timeout"),
                deployment.request_timeout,
            )?,
        });
    }
    Ok(TimeoutSettings {
        global,
        router,
        deployments,
    })
}

pub fn plan(
    settings: &TimeoutSettings,
    request: &RequestTimeouts,
    deployment: usize,
    now: Instant,
) -> Result<TimeoutPlan, RoutingTimeoutError> {
    let params = settings
        .deployments
        .get(deployment)
        .ok_or(RoutingTimeoutError::UnknownDeployment { index: deployment })?;
    Ok(TimeoutPlan::new(now, &settings.router, request, params)?)
}

fn optional(field: &str, value: Option<f64>) -> Result<Option<Seconds>, RoutingTimeoutError> {
    value
        .map(|seconds| Seconds::new(field, seconds))
        .transpose()
        .map_err(RoutingTimeoutError::from)
}
