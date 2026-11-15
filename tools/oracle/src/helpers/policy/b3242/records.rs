use betterlite_policy::context::ResolvedPolicy;
use betterlite_policy::records::attachment::{
    PolicyAttachmentCreateRequest, PolicyAttachmentDBResponse, PolicyAttachmentListResponse,
};
use betterlite_policy::records::crud::{
    PolicyConditionRequest, PolicyCreateRequest, PolicyUpdateRequest,
};
use betterlite_policy::records::db::{PolicyDBResponse, PolicyListDBResponse};
use betterlite_policy::records::inspection::{
    PolicyGuardrailsResponse, PolicyInfoResponse, PolicyListResponse, PolicyScopeResponse,
    PolicySummaryItem, PolicyTestResponse,
};
use betterlite_policy::records::resolution::{
    AttachmentImpactResponse, PipelineTestRequest, PolicyMatchDetail, PolicyResolveRequest,
    PolicyResolveResponse,
};
use betterlite_policy::records::version::{
    PolicyVersionCompareResponse,
    PolicyVersionCreateRequest,
    PolicyVersionListResponse,
    // ast-grep-ignore: no-long-names
    PolicyVersionStatusUpdateRequest as VersionStatusUpdate,
};
use betterlite_policy::values::{Policy, PolicyGuardrails, PolicyScope};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::helper_error;
use crate::helpers::HelperResult;

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Call {
    Validate {
        model: String,
        value: Value,
    },
    ProjectScope {
        scope: Value,
    },
    ProjectGuardrails {
        guardrails: Value,
    },
    ProjectInfo {
        policy_name: String,
        policy: Value,
        resolved: Value,
    },
}

pub(super) fn run(call: &Call) -> HelperResult {
    match call {
        Call::Validate { model, value } => dumped_record(model, value.clone()),
        Call::ProjectScope { scope } => {
            let parsed_scope: PolicyScope = serde_json::from_value(scope.clone())
                .map_err(|error| helper_error("InvalidInput", error.to_string()))?;
            serde_json::to_value(PolicyScopeResponse::from_scope(&parsed_scope))
                .map_err(|error| helper_error("InvalidInput", error.to_string()))
        }
        Call::ProjectGuardrails { guardrails } => {
            let parsed_guardrails: PolicyGuardrails = serde_json::from_value(guardrails.clone())
                .map_err(|error| helper_error("InvalidInput", error.to_string()))?;
            serde_json::to_value(PolicyGuardrailsResponse::from_guardrails(
                &parsed_guardrails,
            ))
            .map_err(|error| helper_error("InvalidInput", error.to_string()))
        }
        Call::ProjectInfo {
            policy_name,
            policy,
            resolved,
        } => {
            let parsed_policy: Policy = serde_json::from_value(policy.clone())
                .map_err(|error| helper_error("InvalidInput", error.to_string()))?;
            let parsed_resolved: ResolvedPolicy = serde_json::from_value(resolved.clone())
                .map_err(|error| helper_error("InvalidInput", error.to_string()))?;
            serde_json::to_value(PolicyInfoResponse::from_policy(
                policy_name,
                &parsed_policy,
                &parsed_resolved,
            ))
            .map_err(|error| helper_error("InvalidInput", error.to_string()))
        }
    }
}

fn dumped<T>(value: Value) -> HelperResult
where
    T: DeserializeOwned + Serialize,
{
    let record: T = serde_json::from_value(value)
        .map_err(|error| helper_error("InvalidInput", error.to_string()))?;
    serde_json::to_value(record).map_err(|error| helper_error("InvalidInput", error.to_string()))
}

fn dumped_record(model: &str, value: Value) -> HelperResult {
    match model {
        "PolicyScopeResponse" => dumped::<PolicyScopeResponse>(value),
        "PolicyGuardrailsResponse" => dumped::<PolicyGuardrailsResponse>(value),
        "PolicyInfoResponse" => dumped::<PolicyInfoResponse>(value),
        "PolicySummaryItem" => dumped::<PolicySummaryItem>(value),
        "PolicyListResponse" => dumped::<PolicyListResponse>(value),
        "PolicyTestResponse" => dumped::<PolicyTestResponse>(value),
        "PolicyConditionRequest" => dumped::<PolicyConditionRequest>(value),
        "PolicyCreateRequest" => dumped::<PolicyCreateRequest>(value),
        "PolicyUpdateRequest" => dumped::<PolicyUpdateRequest>(value),
        "PolicyDBResponse" => dumped::<PolicyDBResponse>(value),
        "PolicyListDBResponse" => dumped::<PolicyListDBResponse>(value),
        "PolicyVersionCreateRequest" => dumped::<PolicyVersionCreateRequest>(value),
        "PolicyVersionStatusUpdateRequest" => dumped::<VersionStatusUpdate>(value),
        "PolicyVersionListResponse" => dumped::<PolicyVersionListResponse>(value),
        "PolicyVersionCompareResponse" => dumped::<PolicyVersionCompareResponse>(value),
        "PolicyAttachmentCreateRequest" => dumped::<PolicyAttachmentCreateRequest>(value),
        "PolicyAttachmentDBResponse" => dumped::<PolicyAttachmentDBResponse>(value),
        "PolicyAttachmentListResponse" => dumped::<PolicyAttachmentListResponse>(value),
        "PipelineTestRequest" => dumped::<PipelineTestRequest>(value),
        "PolicyResolveRequest" => dumped::<PolicyResolveRequest>(value),
        "PolicyResolveResponse" => dumped::<PolicyResolveResponse>(value),
        "PolicyMatchDetail" => dumped::<PolicyMatchDetail>(value),
        "AttachmentImpactResponse" => dumped::<AttachmentImpactResponse>(value),
        _ => Err(helper_error(
            "InvalidInput",
            format!("unknown record model {model}"),
        )),
    }
}
