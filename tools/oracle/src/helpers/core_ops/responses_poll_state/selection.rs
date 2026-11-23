use std::sync::PoisonError;

use betterlite_config::api::responses_poll_state::{
    DEFAULT_POLLING_TTL, PollingSelector, PollingSettings,
};
use betterlite_core::api::responses_poll_state::selection::{
    DeploymentProjection, DeploymentProvider, PollingRequest, should_use_polling_for_request,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use serde::Deserialize;
use serde_json::{Value, json};

use super::invalid;
use super::watch::Watch;

#[derive(Deserialize)]
pub(super) struct Selection {
    background: bool,
    selector: Json,
    redis: bool,
    model: String,
    native_background_mode: Vec<String>,
    router: Option<Router>,
}

#[derive(Deserialize)]
struct Router {
    indices: Vec<usize>,
    deployments: Vec<Deployment>,
}

#[derive(Deserialize)]
struct Deployment {
    custom_llm_provider: Option<String>,
    model: Option<String>,
}

pub(super) fn result(selection: &Selection, watch: &Watch) -> Result<Value, Box<HelperError>> {
    let start = watch
        .lines
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .len();
    let settings = PollingSettings {
        polling_via_cache: selector(&selection.selector)?,
        native_background_mode: selection.native_background_mode.clone(),
        ttl: DEFAULT_POLLING_TTL,
    };
    let providers = deployments(selection);
    let projection = selection
        .router
        .as_ref()
        .map(|router| DeploymentProjection {
            indices: &router.indices,
            deployments: &providers,
        });
    let request = PollingRequest {
        background: selection.background,
        model: &selection.model,
        redis_available: selection.redis,
    };
    let enabled = should_use_polling_for_request(request, &settings, projection.as_ref());
    let logs = {
        let guard = watch.lines.lock().unwrap_or_else(PoisonError::into_inner);
        Value::Array(guard.iter().skip(start).cloned().collect())
    };
    Ok(json!({"enabled": enabled, "logs": logs}))
}

fn deployments(selection: &Selection) -> Vec<DeploymentProvider<'_>> {
    let Some(router) = selection.router.as_ref() else {
        return Vec::new();
    };
    router
        .deployments
        .iter()
        .map(|deployment| DeploymentProvider {
            custom_llm_provider: deployment.custom_llm_provider.as_deref(),
            model: deployment.model.as_deref(),
        })
        .collect()
}

fn selector(value: &Json) -> Result<PollingSelector, Box<HelperError>> {
    match value {
        Json::Null | Json::Bool(false) => Ok(PollingSelector::Disabled),
        Json::Text(text) if text == "all" => Ok(PollingSelector::All),
        Json::Bool(true) | Json::Text(_) | Json::Number(_) | Json::Object(_) => {
            Ok(PollingSelector::Unrecognized)
        }
        Json::List(items) => {
            let mut providers = Vec::with_capacity(items.len());
            for item in items {
                let Json::Text(provider) = item else {
                    return Err(invalid(
                        "polling_via_cache must hold provider names".to_owned(),
                    ));
                };
                providers.push(provider.clone());
            }
            Ok(PollingSelector::Providers(providers))
        }
    }
}
