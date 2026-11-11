use betterlite_config::sections::claude_platform_inputs::{PlatformEnvironment, PlatformInputs};
use secrecy::ExposeSecret as _;

use super::error::PlatformInputError;
use super::{Candidate, optional_candidate, select_matching};
use crate::model_routes::ParamMap;

pub fn resolve_workspace_id(
    optional_params: &ParamMap,
    deployment: &PlatformInputs,
    global: &PlatformInputs,
    environment: &PlatformEnvironment,
) -> Result<Option<String>, PlatformInputError> {
    let selected = resolve_alias(optional_params, deployment, global);
    selected.map_or_else(
        || Ok(environment_workspace(environment)),
        |alias| alias.render().map(Some),
    )
}

fn resolve_alias<'source>(
    optional_params: &'source ParamMap,
    deployment: &'source PlatformInputs,
    global: &'source PlatformInputs,
) -> Option<Candidate<'source>> {
    let primary = select_matching(
        [
            optional_candidate(optional_params, "workspace_id"),
            configured(deployment, global, |inputs| inputs.workspace_id.as_deref()),
            optional_candidate(optional_params, "aws_workspace_id"),
            configured(deployment, global, |inputs| {
                inputs.aws_workspace_id.as_deref()
            }),
            optional_candidate(optional_params, "anthropic-workspace-id"),
            configured(deployment, global, |inputs| {
                inputs.anthropic_hyphen_workspace_id.as_deref()
            }),
        ],
        Candidate::is_effective,
    );
    // LiteLLM reads the underscore alias only when the whole first chain ends at None
    if primary.is_some() {
        return primary;
    }
    select_matching(
        [
            optional_candidate(optional_params, "anthropic_workspace_id"),
            configured(deployment, global, |inputs| {
                inputs.anthropic_workspace_id.as_deref()
            }),
        ],
        Candidate::is_effective,
    )
}

// a deployment value, even an empty one, hides the module value for the same alias
fn configured<'source>(
    deployment: &'source PlatformInputs,
    global: &'source PlatformInputs,
    selector: fn(&'source PlatformInputs) -> Option<&'source str>,
) -> Option<Candidate<'source>> {
    selector(deployment)
        .or_else(|| selector(global))
        .map(Candidate::Text)
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
