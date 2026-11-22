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

use super::watch::Watch;
use super::{Input, invalid};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Selection {
    background: bool,
    selector: Json,
    redis: bool,
    model: String,
    #[serde(default)]
    native_background_mode: Vec<String>,
    #[serde(default)]
    router: Option<Router>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Router {
    indices: Vec<usize>,
    deployments: Vec<Deployment>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Deployment {
    #[serde(default)]
    custom_llm_provider: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

pub(super) fn selections(input: &Input, watch: &Watch) -> Result<Value, Box<HelperError>> {
    let mut results = Vec::with_capacity(input.selections.len());
    for selection in &input.selections {
        results.push(result(selection, watch)?);
    }
    Ok(Value::Array(results))
}

fn result(selection: &Selection, watch: &Watch) -> Result<Value, Box<HelperError>> {
    let start = watch.line_count();
    let settings = PollingSettings {
        polling_via_cache: selector(&selection.selector)?,
        native_background_mode: selection.native_background_mode.clone(),
        ttl: DEFAULT_POLLING_TTL,
    };
    let deployments = selection.router.as_ref().map(|router| {
        router
            .deployments
            .iter()
            .map(|deployment| DeploymentProvider {
                custom_llm_provider: deployment.custom_llm_provider.as_deref(),
                model: deployment.model.as_deref(),
            })
            .collect::<Vec<DeploymentProvider<'_>>>()
    });
    let indices = selection
        .router
        .as_ref()
        .map(|router| router.indices.as_slice());
    let projection = match (indices, deployments.as_deref()) {
        (Some(indices), Some(deployments)) => Some(DeploymentProjection {
            indices,
            deployments,
        }),
        _ => None,
    };
    let enabled = should_use_polling_for_request(
        PollingRequest {
            background: selection.background,
            model: &selection.model,
        },
        &settings,
        selection.redis,
        projection.as_ref(),
    );
    Ok(json!({"enabled": enabled, "logs": watch.lines_since(start)}))
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
