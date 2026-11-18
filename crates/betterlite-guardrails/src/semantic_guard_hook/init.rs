use std::path::PathBuf;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_identity::item::{GuardrailFlag, GuardrailValue};
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use thiserror::Error;

use crate::guardrail_initializers::error::InitializerError;
use crate::guardrail_initializers::extras;
use crate::semantic_guard::semantic_guard_route_loading::default_similarity_threshold;
use crate::semantic_guard::semantic_guard_route_loading::error::RouteLoadError;

mod tests;

pub const EMBEDDING_MODEL_VARIABLE: &str = "DEFAULT_SEMANTIC_GUARD_EMBEDDING_MODEL";
pub const DEFAULT_EMBEDDING_MODEL: &str = "text-embedding-3-small";
pub const DEFAULT_ON_FLAGGED_ACTION: &str = "block";

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticGuardArgs {
    pub name: String,
    pub embedding_model: String,
    pub similarity_threshold: f64,
    pub route_templates: Vec<String>,
    pub custom_routes_file: Option<PathBuf>,
    pub custom_routes: Vec<GuardrailValue>,
    pub on_flagged_action: String,
    pub event_hook: Option<LitellmParamsMode>,
    pub default_on: GuardrailFlag,
}

#[derive(Debug, Error)]
pub enum SemanticGuardInitError {
    #[error("SemanticGuard: guardrail_name is required")]
    MissingName,
    #[error(
        "SemanticGuard requires llm_router for embeddings. Configure a model_list with an embedding model."
    )]
    MissingRouter,
    #[error(transparent)]
    Params(#[from] InitializerError),
    #[error(transparent)]
    Threshold(#[from] RouteLoadError),
}

pub fn initialize_semantic_guard(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
    router_present: bool,
    environment: &dyn EnvironmentLookup,
) -> Result<SemanticGuardArgs, SemanticGuardInitError> {
    let Some(name) = guardrail_name.filter(|name| !name.is_empty()) else {
        return Err(SemanticGuardInitError::MissingName);
    };
    if !router_present {
        return Err(SemanticGuardInitError::MissingRouter);
    }
    let extra = &params.base.extra;
    // the import-time constant LiteLLM builds from the environment, where an empty value counts as unset
    let environment_model = environment
        .lookup(EMBEDDING_MODEL_VARIABLE)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_EMBEDDING_MODEL.to_owned());
    let embedding_model =
        extras::read::<String>(extra, "embedding_model", None)?.unwrap_or(environment_model);
    let default_threshold = default_similarity_threshold(environment)?;
    let similarity_threshold = extras::read::<f64>(extra, "similarity_threshold", None)?
        .filter(|value| *value != 0.0_f64)
        .unwrap_or(default_threshold);
    let route_templates =
        extras::read::<Vec<String>>(extra, "route_templates", None)?.unwrap_or_default();
    let custom_routes_file =
        extras::read::<String>(extra, "custom_routes_file", None)?.map(PathBuf::from);
    let custom_routes =
        extras::read::<Vec<GuardrailValue>>(extra, "custom_routes", None)?.unwrap_or_default();
    let on_flagged_action = extras::read::<String>(extra, "on_flagged_action", None)?
        .unwrap_or_else(|| DEFAULT_ON_FLAGGED_ACTION.to_owned());
    Ok(SemanticGuardArgs {
        name: name.to_owned(),
        embedding_model,
        similarity_threshold,
        route_templates,
        custom_routes_file,
        custom_routes,
        on_flagged_action,
        event_hook: Some(params.mode.clone()),
        default_on: params.base.fields.default_on.unwrap_or_default(),
    })
}
