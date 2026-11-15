use betterlite_models::decisions::Json;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::passthrough_selection::error::PassthroughSelectionError;
use crate::simple_shuffle::Candidate;

// use_in_pass_through is declared as a bool: absent and null are false, and any other type names the field.
pub(super) fn pass_through_flag(
    deployment: &DeploymentSpec,
) -> Result<bool, PassthroughSelectionError> {
    match deployment.litellm_params.get("use_in_pass_through") {
        Some(Json::Bool(value)) => Ok(*value),
        Some(Json::Null) | None => Ok(false),
        Some(Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => Err(
            invalid_field(deployment, "use_in_pass_through", "a boolean"),
        ),
    }
}

pub(super) fn metrics(deployment: &DeploymentSpec) -> Result<Candidate, PassthroughSelectionError> {
    Ok(Candidate {
        weight: metric(deployment, "weight")?,
        rpm: metric(deployment, "rpm")?,
        tpm: metric(deployment, "tpm")?,
    })
}

// _metric_weight: absent and null weigh nothing, and a value outside a 32-bit count cannot be weighed.
fn metric(
    deployment: &DeploymentSpec,
    field: &'static str,
) -> Result<u32, PassthroughSelectionError> {
    match deployment.litellm_params.get(field) {
        Some(Json::Number(number)) => number
            .as_u64()
            .and_then(|count| u32::try_from(count).ok())
            .ok_or_else(|| {
                invalid_field(deployment, field, "a non-negative whole number below 2^32")
            }),
        Some(Json::Null) | None => Ok(0),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => Err(
            invalid_field(deployment, field, "a non-negative whole number below 2^32"),
        ),
    }
}

fn invalid_field(
    deployment: &DeploymentSpec,
    field: &'static str,
    expected: &'static str,
) -> PassthroughSelectionError {
    PassthroughSelectionError::InvalidDeploymentField {
        deployment_id: deployment.id().unwrap_or_default().to_owned(),
        field,
        expected,
    }
}
