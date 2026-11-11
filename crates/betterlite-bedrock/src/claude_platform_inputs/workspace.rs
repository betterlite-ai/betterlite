use betterlite_config::sections::claude_platform_inputs::{PlatformEnvironment, PlatformInputs};
use secrecy::ExposeSecret as _;

use super::error::PlatformInputError;
use super::{Candidate, select};
use crate::model_routes::{ParamMap, ParamValue};

pub fn resolve_workspace_id(
    optional_params: &ParamMap,
    deployment: &PlatformInputs,
    global: &PlatformInputs,
    environment: &PlatformEnvironment,
) -> Result<Option<String>, PlatformInputError> {
    let selected = primary_alias(optional_params, deployment, global)
        .or_else(|| secondary_alias(optional_params, deployment, global));
    selected.map_or_else(
        || Ok(environment_workspace(environment)),
        |alias| alias.render().map(Some),
    )
}

fn primary_alias<'source>(
    optional_params: &'source ParamMap,
    deployment: &'source PlatformInputs,
    global: &'source PlatformInputs,
) -> Option<Candidate<'source>> {
    select([
        optional_alias(optional_params, "workspace_id", "workspace_id"),
        deployment
            .workspace_id
            .as_deref()
            .or(global.workspace_id.as_deref())
            .map(Candidate::Text),
        optional_alias(optional_params, "aws_workspace_id", "aws_workspace_id"),
        deployment
            .aws_workspace_id
            .as_deref()
            .or(global.aws_workspace_id.as_deref())
            .map(Candidate::Text),
        optional_alias(
            optional_params,
            "anthropic-workspace-id",
            "anthropic-workspace-id",
        ),
        deployment
            .anthropic_hyphen_workspace_id
            .as_deref()
            .or(global.anthropic_hyphen_workspace_id.as_deref())
            .map(Candidate::Text),
    ])
}

// a deployment value, even an empty one, hides the module value for the same alias
fn secondary_alias<'source>(
    optional_params: &'source ParamMap,
    deployment: &'source PlatformInputs,
    global: &'source PlatformInputs,
) -> Option<Candidate<'source>> {
    select([
        optional_alias(
            optional_params,
            "anthropic_workspace_id",
            "anthropic_workspace_id",
        ),
        deployment
            .anthropic_workspace_id
            .as_deref()
            .or(global.anthropic_workspace_id.as_deref())
            .map(Candidate::Text),
    ])
}

fn optional_alias<'source>(
    params: &'source ParamMap,
    field: &'static str,
    key: &str,
) -> Option<Candidate<'source>> {
    params
        .get(key)
        .filter(|value| !matches!(value, ParamValue::Null))
        .map(|value| Candidate::Param { field, value })
}

fn environment_workspace(environment: &PlatformEnvironment) -> Option<String> {
    let first = environment
        .anthropic_aws_workspace_id
        .as_ref()
        .filter(|value| !value.expose_secret().is_empty())
        .map(|value| value.expose_secret().to_owned());
    first.or_else(|| {
        environment
            .anthropic_workspace_id
            .as_ref()
            .map(|value| value.expose_secret().to_owned())
    })
}
