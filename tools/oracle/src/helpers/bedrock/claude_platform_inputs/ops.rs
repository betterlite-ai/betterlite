use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;

use betterlite_bedrock::claude_platform_inputs::CLAUDE_PLATFORM_BEDROCK_ROUTE;
use betterlite_bedrock::claude_platform_inputs::body::{
    PlatformWarning, filter_request_body, resolve_unsupported_override,
};
use betterlite_bedrock::claude_platform_inputs::endpoint::{
    UrlInputs, complete_url, resolve_region,
};
use betterlite_bedrock::claude_platform_inputs::error::PlatformInputError;
use betterlite_bedrock::claude_platform_inputs::workspace::resolve_workspace_id;
use betterlite_config::loader::ConfigLoader;
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
    ConfigLoader {
        lookup: case.environment.clone(),
    }
    .load(&source)
    .map_err(invalid)
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

fn string_outcome(key: &str, outcome: Result<String, PlatformInputError>) -> Value {
    match outcome {
        Ok(value) => json!({key: value}),
        Err(error) => json!({"error": match error {
            PlatformInputError::MissingRegion => "missing_region",
            PlatformInputError::Region(_) => "invalid_region",
            PlatformInputError::ValueType { .. } | PlatformInputError::Signing(_) => "other",
        }}),
    }
}

fn regions(cases: &[RegionCase]) -> HelperResult {
    cases
        .iter()
        .map(|case| {
            let config = config(&case.environment)?;
            Ok(string_outcome(
                "region",
                resolve_region(
                    &case.optional_params,
                    &config.sections.bedrock_aws_endpoints,
                ),
            ))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|outcomes| json!({"regions": outcomes}))
}

fn urls(cases: &[UrlCase]) -> HelperResult {
    let mut outcomes = Vec::with_capacity(cases.len());
    for case in cases {
        let config = config(&case.environment)?;
        let outcome = complete_url(
            &UrlInputs {
                api_base: case.api_base.as_deref(),
                litellm_api_base: case.litellm_api_base.as_deref(),
                optional_params: &case.optional_params,
            },
            &config.sections.claude_platform_inputs.environment,
            &config.sections.bedrock_aws_endpoints,
        );
        outcomes.push(string_outcome("url", outcome));
    }
    Ok(json!({"urls": outcomes}))
}

fn route_models(models: &[String]) -> Value {
    let stripped: Vec<String> = models
        .iter()
        .map(|model| {
            model
                .strip_prefix(CLAUDE_PLATFORM_BEDROCK_ROUTE)
                .unwrap_or(model)
                .to_owned()
        })
        .collect();
    json!({"models": stripped})
}

fn overrides(calls: &[OverrideCall]) -> Value {
    let mut outcomes = Vec::with_capacity(calls.len());
    for call in calls {
        let resolution = resolve_unsupported_override(
            call.deployment.as_ref(),
            call.optional.as_ref(),
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
