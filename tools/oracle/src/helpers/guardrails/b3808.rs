use std::fs;
use std::path::{Path, PathBuf};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::semantic_guard::semantic_guard_route_loading::DEFAULT_SIMILARITY_THRESHOLD;
use betterlite_guardrails::semantic_guard::semantic_guard_route_loading::build_routes;
use betterlite_guardrails::semantic_guard::semantic_guard_route_loading::sources::RouteSources;
use betterlite_guardrails::semantic_guard::semantic_guard_route_loading::templates;
use serde::Deserialize;
use serde_json::json;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    BuildRoutes(BuildRoutes),
    ListTemplates(ListTemplates),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BuildRoutes {
    #[serde(default)]
    route_templates: Vec<String>,
    #[serde(default)]
    custom_routes_file: Option<String>,
    #[serde(default)]
    missing_routes_file: bool,
    #[serde(default)]
    custom_routes: Vec<GuardrailValue>,
    #[serde(default)]
    global_threshold: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListTemplates {}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::BuildRoutes(case) => build(case),
            Input::ListTemplates(_) => Ok(json!({"templates": templates::list()})),
        }
    }
}

fn build(case: &BuildRoutes) -> HelperResult {
    let directory = tempfile::tempdir()?;
    let sources = RouteSources {
        route_templates: case.route_templates.clone(),
        custom_routes_file: routes_path(case, directory.path())?,
        custom_routes: case.custom_routes.clone(),
        global_threshold: case
            .global_threshold
            .unwrap_or(DEFAULT_SIMILARITY_THRESHOLD),
    };
    let routes = build_routes(&sources).or_value_error()?;
    Ok(json!({"routes": routes}))
}

fn routes_path(case: &BuildRoutes, root: &Path) -> Result<Option<PathBuf>, Box<HelperError>> {
    if case.missing_routes_file {
        return Ok(Some(root.join("absent-routes.yaml")));
    }
    let Some(text) = case.custom_routes_file.as_deref() else {
        return Ok(None);
    };
    let path = root.join("custom-routes.yaml");
    fs::write(&path, text)?;
    Ok(Some(path))
}
