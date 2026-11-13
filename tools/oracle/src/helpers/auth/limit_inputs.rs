mod input;
mod warnings;

use std::collections::BTreeMap;

use betterlite_auth::auth_limit_inputs::batch::{
    BatchEnqueuedTokenLimitRequest, enforce_batch_enqueued_token_limit_is_admin_only,
};
use betterlite_auth::auth_limit_inputs::deployment::{DeploymentLimitCatalog, DeploymentLimits};
use betterlite_auth::auth_limit_inputs::error::{AuthLimitInputsError, ManagementEntity};
use betterlite_auth::auth_limit_inputs::estimates::{
    OutputTokenEstimateRequest, enforce_output_token_estimates_are_admin_only,
    get_estimated_output_tokens,
};
use betterlite_auth::auth_limit_inputs::key_limits::{
    ModelRateLimitKey, get_key_model_rpm_limit, get_key_model_tpm_limit,
};
use betterlite_auth::auth_limit_inputs::metadata_limits::{
    MetadataAccessor, get_key_mcp_rpm_limit, get_key_tag_rpm_limit,
    get_model_rate_limit_from_metadata, get_team_mcp_rpm_limit,
};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use serde_json::json;
use tracing_subscriber::layer::SubscriberExt as _;

use crate::helpers::{Helper, HelperError, HelperResult, OrHelperError as _};

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
            } => enforce_estimates(request, existing_metadata.as_ref(), caller, *entity),
            Input::EnforceBatchLimit {
                request,
                existing_metadata,
                caller,
                entity,
            } => enforce_batch(request, existing_metadata.as_ref(), caller, *entity),
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
    let auth = decode_auth(fields)?;
    let catalog = Catalog { deployments };
    let rpm = get_key_model_rpm_limit(&auth, model_name, Some(&catalog)).or_value_error()?;
    let tpm = get_key_model_tpm_limit(&auth, model_name, Some(&catalog)).or_value_error()?;
    Ok(json!({ "rpm": rpm, "tpm": tpm }))
}

fn estimated_output_tokens(
    fields: &BTreeMap<String, JsonData>,
    model_name: Option<&str>,
) -> HelperResult {
    let auth = decode_auth(fields)?;
    let captured = WarningLog::default();
    let subscriber = tracing_subscriber::registry().with(captured.clone());
    let estimate = tracing::subscriber::with_default(subscriber, || {
        get_estimated_output_tokens(&auth, model_name)
    });
    Ok(json!({ "estimate": estimate, "warnings": captured.warnings() }))
}

fn enforce_estimates(
    fields: &OutputEstimateRequestInput,
    existing_metadata: Option<&BTreeMap<String, JsonData>>,
    caller_fields: &BTreeMap<String, JsonData>,
    entity: EntityInput,
) -> HelperResult {
    let caller = decode_auth(caller_fields)?;
    let request = OutputTokenEstimateRequest {
        metadata: fields.metadata.as_ref(),
        default_estimated_output_tokens: fields.default_estimated_output_tokens,
        default_estimated_output_tokens_per_model: fields
            .default_estimated_output_tokens_per_model
            .as_ref(),
        model_fields_set: &fields.model_fields_set,
    };
    match enforce_output_token_estimates_are_admin_only(
        &request,
        existing_metadata,
        &caller,
        ManagementEntity::from(entity),
    ) {
        Ok(()) => Ok(json!({ "ok": true })),
        Err(failure) => forbidden(failure),
    }
}

fn enforce_batch(
    fields: &BatchRequestInput,
    existing_metadata: Option<&BTreeMap<String, JsonData>>,
    caller_fields: &BTreeMap<String, JsonData>,
    entity: EntityInput,
) -> HelperResult {
    let caller = decode_auth(caller_fields)?;
    let request = BatchEnqueuedTokenLimitRequest {
        metadata: fields.metadata.as_ref(),
        model_fields_set: &fields.model_fields_set,
    };
    match enforce_batch_enqueued_token_limit_is_admin_only(
        &request,
        existing_metadata,
        &caller,
        ManagementEntity::from(entity),
    ) {
        Ok(()) => Ok(json!({ "ok": true })),
        Err(failure) => forbidden(failure),
    }
}

fn forbidden(failure: AuthLimitInputsError) -> HelperResult {
    match &failure {
        AuthLimitInputsError::OutputTokenEstimatesAdminOnly { .. }
        | AuthLimitInputsError::BatchEnqueuedTokenLimitAdminOnly { .. } => {
            Ok(json!({ "ok": false, "status": 403, "detail_error": true }))
        }
        AuthLimitInputsError::LimitOverflow { .. } => Err(failure).or_value_error(),
    }
}

fn tenant_fields(fields: &BTreeMap<String, JsonData>) -> HelperResult {
    let auth = decode_auth(fields)?;
    Ok(json!({
        "team_rpm": get_model_rate_limit_from_metadata(&auth, MetadataAccessor::Team, ModelRateLimitKey::Rpm),
        "organization_itpm": get_model_rate_limit_from_metadata(&auth, MetadataAccessor::Organization, ModelRateLimitKey::Itpm),
        "organization_otpm": get_model_rate_limit_from_metadata(&auth, MetadataAccessor::Organization, ModelRateLimitKey::Otpm),
        "project_tpm": get_model_rate_limit_from_metadata(&auth, MetadataAccessor::Project, ModelRateLimitKey::Tpm),
        "key_mcp": get_key_mcp_rpm_limit(&auth),
        "team_mcp": get_team_mcp_rpm_limit(&auth),
        "tag": get_key_tag_rpm_limit(&auth),
    }))
}

fn mcp_rpm_limit(fields: &BTreeMap<String, JsonData>, team_only: bool) -> HelperResult {
    let auth = decode_auth(fields)?;
    let limits = if team_only {
        get_team_mcp_rpm_limit(&auth)
    } else {
        get_key_mcp_rpm_limit(&auth)
    };
    Ok(json!({ "limits": limits }))
}

fn tag_rpm_limit(fields: &BTreeMap<String, JsonData>) -> HelperResult {
    let auth = decode_auth(fields)?;
    Ok(json!({ "limits": get_key_tag_rpm_limit(&auth) }))
}

fn decode_auth(fields: &BTreeMap<String, JsonData>) -> Result<UserAPIKeyAuth, Box<HelperError>> {
    UserAPIKeyAuth::construct(AuthInput::Dict(fields.clone())).or_value_error()
}
