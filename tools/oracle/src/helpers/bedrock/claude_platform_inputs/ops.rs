use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;

use betterlite_bedrock::claude_platform_inputs::body::{
    PlatformWarning, filter_request_body, resolve_unsupported_override,
};
use betterlite_bedrock::claude_platform_inputs::endpoint::{
    UrlInputs, complete_url, resolve_region,
};
use betterlite_bedrock::claude_platform_inputs::error::PlatformInputError;
use betterlite_bedrock::claude_platform_inputs::route::strip_claude_platform_route;
use betterlite_bedrock::claude_platform_inputs::workspace::resolve_workspace_id;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::claude_platform_inputs::{PlatformValue, UnsupportedParams};
use betterlite_config::types::Config;
use serde_json::{Value, json};

use super::input::{BodyCall, Input, OverrideCall, RegionCase, UrlCase, WorkspaceCase};
use super::signing::signing;
use crate::helpers::{HelperError, HelperResult};

const FIXTURE_CONFIG: &str =
    "model_list:\n- model_name: unit\n  litellm_params: {model: bedrock/unit}\n";

pub(super) fn invalid<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

pub(super) fn config(environment: &BTreeMap<String, String>) -> Result<Config, Box<HelperError>> {
    ConfigLoader {
        lookup: environment.clone(),
    }
    .load(FIXTURE_CONFIG)
    .map_err(invalid)
}

fn config_from(
    source: &str,
    environment: &BTreeMap<String, String>,
) -> Result<Config, Box<HelperError>> {
    ConfigLoader {
        lookup: environment.clone(),
    }
    .load(source)
    .map_err(invalid)
}

const fn error_code(error: &PlatformInputError) -> &'static str {
    match error {
        PlatformInputError::MissingRegion => "missing_region",
        PlatformInputError::Region(_) => "invalid_region",
        PlatformInputError::ValueType { .. } | PlatformInputError::Signing(_) => "other",
    }
}

fn warning_json(warning: &PlatformWarning) -> Value {
    match warning {
        PlatformWarning::DroppedUnsupported { params } => {
            json!({"action": "drop", "params": params})
        }
        PlatformWarning::IgnoredOverride { type_name } => {
            json!({"action": "ignore", "type_name": type_name})
        }
    }
}

fn workspace_config(case: &WorkspaceCase) -> Result<Config, Box<HelperError>> {
    let mut params = serde_json::Map::new();
    params.insert("model".to_owned(), Value::String("bedrock/unit".to_owned()));
    for (key, value) in &case.deployment {
        params.insert(key.clone(), Value::String(value.clone()));
    }
    let document = json!({
        "model_list": [{"model_name": "unit", "litellm_params": params}],
        "litellm_settings": case.litellm_settings,
    });
    let source = serde_saphyr::to_string(&document).map_err(invalid)?;
    config_from(&source, &case.environment)
}

fn workspaces(cases: &[WorkspaceCase]) -> HelperResult {
    let mut outcomes = Vec::with_capacity(cases.len());
    for case in cases {
        let config = workspace_config(case)?;
        let settings = &config.sections.claude_platform_inputs;
        let deployment = settings.deployments.first().cloned().unwrap_or_default();
        let workspace = resolve_workspace_id(
            &case.optional_params,
            &deployment,
            &settings.litellm_settings,
            &settings.environment,
        )
        .map_err(invalid)?;
        outcomes.push(workspace);
    }
    Ok(json!({"workspaces": outcomes}))
}

fn region_outcome(outcome: Result<String, PlatformInputError>) -> Value {
    match outcome {
        Ok(region) => json!({"region": region}),
        Err(error) => json!({"error": error_code(&error)}),
    }
}

fn regions(cases: &[RegionCase]) -> HelperResult {
    cases
        .iter()
        .map(|case| {
            let config = config(&case.environment)?;
            Ok(region_outcome(resolve_region(
                &case.optional_params,
                &config.sections.bedrock_aws_endpoints,
            )))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|outcomes| json!({"regions": outcomes}))
}

fn url_case(case: &UrlCase, config: &Config) -> Value {
    let outcome = complete_url(
        &UrlInputs {
            api_base: case.api_base.as_deref(),
            litellm_api_base: case.litellm_api_base.as_deref(),
            optional_params: &case.optional_params,
        },
        &config.sections.claude_platform_inputs.environment,
        &config.sections.bedrock_aws_endpoints,
    );
    match outcome {
        Ok(url) => json!({"url": url}),
        Err(error) => json!({"error": error_code(&error)}),
    }
}

fn urls(cases: &[UrlCase]) -> HelperResult {
    let mut outcomes = Vec::with_capacity(cases.len());
    for case in cases {
        outcomes.push(url_case(case, &config(&case.environment)?));
    }
    Ok(json!({"urls": outcomes}))
}

fn route_models(models: &[String]) -> Value {
    let stripped: Vec<String> = models
        .iter()
        .map(|model| strip_claude_platform_route(model))
        .collect();
    json!({"models": stripped})
}

// LiteLLM reads a null override as absent, so an explicit null never warns
const fn absent_override(value: Option<&UnsupportedParams>) -> Option<&UnsupportedParams> {
    match value {
        Some(UnsupportedParams::Other(PlatformValue::Null)) => None,
        _ => value,
    }
}

fn overrides(calls: &[OverrideCall]) -> Value {
    let mut outcomes = Vec::with_capacity(calls.len());
    for call in calls {
        let resolution = resolve_unsupported_override(
            absent_override(call.deployment.as_ref()),
            absent_override(call.optional.as_ref()),
            call.log_invalid,
        );
        outcomes.push(json!({
            "names": resolution.names.map(|names| names.into_iter().collect::<Vec<_>>()),
            "warning": resolution.warning.as_ref().map_or(Value::Null, warning_json),
        }));
    }
    json!({"calls": outcomes})
}

fn bodies(calls: &[BodyCall]) -> HelperResult {
    let mut outcomes = Vec::with_capacity(calls.len());
    for call in calls {
        let unsupported: Option<BTreeSet<String>> = call
            .unsupported
            .as_ref()
            .map(|names| names.iter().cloned().collect());
        let filtered = filter_request_body(&call.params, unsupported.as_ref(), call.log_dropped);
        let warnings: Vec<Value> = filtered.warning.iter().map(warning_json).collect();
        outcomes.push(json!({
            "body": serde_json::to_value(&filtered.body).map_err(invalid)?,
            "warnings": warnings,
        }));
    }
    Ok(json!({"calls": outcomes}))
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::WorkspaceAliasPrecedence { cases } => workspaces(cases),
        Input::ExplicitRegionRequired { cases } => regions(cases),
        Input::ApiBaseBeforeRegion { cases } => urls(cases),
        Input::PlatformRoutePrefix { models } => Ok(route_models(models)),
        Input::UnsupportedOverride { calls } => Ok(overrides(calls)),
        Input::FilteredBodyFields { calls } => bodies(calls),
        Input::PlatformSigning { calls } => signing(calls),
    }
}
