use betterlite_auth::auth_identity::records::Principal;

use crate::api::file_routing_ids::ManagedResourceAccessChecker;
use crate::api::file_upload_options::error::{FileUploadOptionsError, ResourceKind};
use crate::api::managed_resource_ids::is_base64_encoded_unified_id;

pub const MANAGED_FILE_ID_PREFIX: &str = "litellm_proxy";

pub const fn validate_managed_upload(
    require_managed_files: bool,
    target_model_names: &[String],
    model: Option<&str>,
) -> Result<(), FileUploadOptionsError> {
    if !require_managed_files {
        return Ok(());
    }
    if target_model_names.is_empty() {
        return Err(FileUploadOptionsError::TargetModelNamesRequired);
    }
    if matches!(model, Some(text) if !text.is_empty()) {
        return Err(FileUploadOptionsError::ModelNotAllowed);
    }
    Ok(())
}

pub async fn validate_managed_id(
    require_managed_files: bool,
    resource_id: Option<&str>,
    resource_kind: ResourceKind,
    checker: Option<&dyn ManagedResourceAccessChecker>,
    principal: &Principal,
) -> Result<(), FileUploadOptionsError> {
    if !require_managed_files {
        return Ok(());
    }
    let Some(id) = resource_id.filter(|candidate| !candidate.is_empty()) else {
        return Ok(());
    };
    if is_base64_encoded_unified_id(id, MANAGED_FILE_ID_PREFIX).is_none() {
        return Err(FileUploadOptionsError::RawProviderId {
            kind: resource_kind,
        });
    }
    let Some(access_checker) = checker else {
        return Err(FileUploadOptionsError::OwnershipCheckUnavailable);
    };
    let allowed = match resource_kind {
        ResourceKind::File => access_checker.can_call_unified_file_id(id, principal).await,
        ResourceKind::Batch | ResourceKind::FineTuningJob => {
            access_checker
                .can_call_unified_object_id(id, principal)
                .await
        }
    };
    if allowed {
        return Ok(());
    }
    Err(FileUploadOptionsError::AccessDenied {
        kind: resource_kind,
    })
}
