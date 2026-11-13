mod input;
mod warnings;

use std::collections::BTreeMap;

use betterlite_auth::auth_limit_inputs::batch::{BatchLimitRequest, enforce_batch_admin_only};
use betterlite_auth::auth_limit_inputs::deployment::{DeploymentLimitCatalog, DeploymentLimits};
use betterlite_auth::auth_limit_inputs::error::{AuthLimitInputsError, ManagementEntity};
use betterlite_auth::auth_limit_inputs::estimates::{
    OutputTokenEstimateRequest, enforce_estimate_admin_only, get_estimated_output_tokens,
};
use betterlite_auth::auth_limit_inputs::key_limits::{
    ModelLimitQuery, ModelRateLimitKey, get_key_model_rpm_limit, get_key_model_tpm_limit,
};
use betterlite_auth::auth_limit_inputs::metadata_limits::{
    MetadataAccessor, get_key_mcp_rpm_limit, get_key_tag_rpm_limit, get_team_mcp_rpm_limit,
    model_limit_from_metadata,
};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use serde_json::json;
use tracing_subscriber::layer::SubscriberExt as _;

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

use input::{BatchRequestInput, DeploymentInput, EntityInput, Input, OutputEstimateRequestInput};
use warnings::WarningLog;

struct Catalog<'input> {
    deployments: Option<&'input [DeploymentInput]>,
}

impl DeploymentLimitCatalog for Catalog<'_> {
    fn model_list(&self, model_name: &str) -> Option<Vec<DeploymentLimits>> {
        let deployments = self.deployments?;
        Some(
            deployments
                .iter()
                .filter(|deployment| deployment.model_name == model_name)
                .map(DeploymentInput::limits)
                .collect(),
        )
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::KeyModelLimits {
                auth,
                model_name,
                deployments,
            } => key_model_limits(auth, model_name.as_deref(), deployments.as_deref()),
            Input::EstimatedOutputTokens { auth, model_name } => {
                estimated_output_tokens(auth, model_name.as_deref())
            }
            Input::EnforceOutputEstimates {
                request,
                existing_metadata,
                caller,
                entity,
            } => enforce(
                &estimate_projection(request),
                existing_metadata.as_ref(),
                caller,
                *entity,
            ),
            Input::EnforceBatchLimit {
                request,
                existing_metadata,
                caller,
                entity,
            } => enforce(
                &batch_projection(request),
                existing_metadata.as_ref(),
                caller,
                *entity,
            ),
            Input::McpRpmLimit { auth, team_only } => mcp_rpm_limit(auth, *team_only),
            Input::TagRpmLimit { auth } => tag_rpm_limit(auth),
            Input::TenantFields { auth } => tenant_fields(auth),
        }
    }
}

fn key_model_limits(
    fields: &BTreeMap<String, JsonData>,
    model_name: Option<&str>,
    deployments: Option<&[DeploymentInput]>,
) -> HelperResult {
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(fields.clone())).or_value_error()?;
    let catalog = Catalog { deployments };
    let query = ModelLimitQuery {
        auth: &auth,
        model_name,
        catalog: Some(&catalog),
    };
    let rpm = get_key_model_rpm_limit(&query).or_value_error()?;
    let tpm = get_key_model_tpm_limit(&query).or_value_error()?;
    Ok(json!({ "rpm": rpm, "tpm": tpm }))
}

fn estimated_output_tokens(
    fields: &BTreeMap<String, JsonData>,
    model_name: Option<&str>,
) -> HelperResult {
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(fields.clone())).or_value_error()?;
    let captured = WarningLog::default();
    let subscriber = tracing_subscriber::registry().with(captured.clone());
    let estimate = tracing::subscriber::with_default(subscriber, || {
        get_estimated_output_tokens(&auth, model_name)
    });
    Ok(json!({ "estimate": estimate, "warnings": captured.warnings() }))
}

enum Projection<'request> {
    Estimate(OutputTokenEstimateRequest<'request>),
    Batch(BatchLimitRequest<'request>),
}

fn estimate_projection(fields: &OutputEstimateRequestInput) -> Projection<'_> {
    Projection::Estimate(OutputTokenEstimateRequest {
        metadata: fields.metadata.as_ref(),
        estimate: fields.estimate,
        estimate_per_model: fields.estimate_per_model.as_ref(),
        model_fields_set: &fields.model_fields_set,
    })
}

fn batch_projection(fields: &BatchRequestInput) -> Projection<'_> {
    Projection::Batch(BatchLimitRequest {
        metadata: fields.metadata.as_ref(),
        model_fields_set: &fields.model_fields_set,
    })
}

fn enforce(
    projection: &Projection<'_>,
    existing_metadata: Option<&BTreeMap<String, JsonData>>,
    caller_fields: &BTreeMap<String, JsonData>,
    entity: EntityInput,
) -> HelperResult {
    let caller =
        UserAPIKeyAuth::construct(AuthInput::Dict(caller_fields.clone())).or_value_error()?;
    let result = match projection {
        Projection::Estimate(request) => enforce_estimate_admin_only(
            request,
            existing_metadata,
            &caller,
            ManagementEntity::from(entity),
        ),
        Projection::Batch(request) => enforce_batch_admin_only(
            request,
            existing_metadata,
            &caller,
            ManagementEntity::from(entity),
        ),
    };
    outcome(result)
}

fn outcome(result: Result<(), AuthLimitInputsError>) -> HelperResult {
    match result {
        Ok(()) => Ok(json!({ "ok": true })),
        Err(failure) => forbidden(failure),
    }
}

fn forbidden(failure: AuthLimitInputsError) -> HelperResult {
    match &failure {
        AuthLimitInputsError::EstimateAdminOnly { .. }
        | AuthLimitInputsError::BatchLimitAdminOnly { .. } => {
            Ok(json!({ "ok": false, "status": 403, "detail_error": true }))
        }
        AuthLimitInputsError::LimitOverflow { .. } => Err(failure).or_value_error(),
    }
}

fn tenant_fields(fields: &BTreeMap<String, JsonData>) -> HelperResult {
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(fields.clone())).or_value_error()?;
    Ok(json!({
        "team_rpm": model_limit_from_metadata(&auth, MetadataAccessor::Team, ModelRateLimitKey::Rpm),
        "organization_itpm": model_limit_from_metadata(&auth, MetadataAccessor::Organization, ModelRateLimitKey::Itpm),
        "organization_otpm": model_limit_from_metadata(&auth, MetadataAccessor::Organization, ModelRateLimitKey::Otpm),
        "project_tpm": model_limit_from_metadata(&auth, MetadataAccessor::Project, ModelRateLimitKey::Tpm),
        "key_mcp": get_key_mcp_rpm_limit(&auth),
        "team_mcp": get_team_mcp_rpm_limit(&auth),
        "tag": get_key_tag_rpm_limit(&auth),
    }))
}

fn mcp_rpm_limit(fields: &BTreeMap<String, JsonData>, team_only: bool) -> HelperResult {
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(fields.clone())).or_value_error()?;
    let limits = if team_only {
        get_team_mcp_rpm_limit(&auth)
    } else {
        get_key_mcp_rpm_limit(&auth)
    };
    Ok(json!({ "limits": limits }))
}

fn tag_rpm_limit(fields: &BTreeMap<String, JsonData>) -> HelperResult {
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(fields.clone())).or_value_error()?;
    Ok(json!({
        "limits": get_key_tag_rpm_limit(&auth),
    }))
}
