use std::sync::Arc;

use betterlite_auth::user_roles::UserRole;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::batches::records::BatchObject;
use rust_decimal::Decimal;
use secrecy::SecretString;
use tokio::runtime::Handle;

use super::error::VertexBatchError;
use crate::pass_through_logging::anthropic_batch::{
    BatchRegistrant, BatchRegistration, ManagedBatchRegistry,
};
use crate::pass_through_logging::batch_attribution::{
    log_batch_registration_result, optional_str, request_tags_from_metadata,
};

pub(super) fn request(
    unified_object_id: &str,
    batch_object: BatchObject,
    request_metadata: &JsonObject,
    is_batch_create: bool,
) -> BatchRegistration {
    let text = |name: &str| request_metadata.get(name).and_then(optional_str);
    let user_id = request_metadata.get("user_api_key_user_id").map_or_else(
        || Some("default-user".to_owned()),
        |value| optional_str(value).map(str::to_owned),
    );
    BatchRegistration {
        unified_object_id: unified_object_id.to_owned(),
        model_object_id: batch_object.id.clone(),
        batch_object,
        file_purpose: "batch".to_owned(),
        registrant: BatchRegistrant {
            user_id,
            api_key: text("user_api_key").map(|key| SecretString::from(key.to_owned())),
            team_id: text("user_api_key_team_id").map(str::to_owned),
            team_alias: None,
            user_role: UserRole::Customer,
            user_email: None,
            max_budget: None,
            spend: Decimal::ZERO,
            models: Vec::new(),
            tpm_limit: None,
            rpm_limit: None,
            budget_duration: None,
            budget_reset_at: None,
            max_parallel_requests: None,
            allowed_model_region: None,
            metadata: JsonObject::new(),
            key_alias: None,
            permissions: JsonObject::new(),
            model_max_budget: JsonObject::new(),
            model_spend: JsonObject::new(),
        },
        request_tags: request_tags_from_metadata(request_metadata),
        persist_attribution: is_batch_create,
        create_if_missing: is_batch_create,
    }
}

pub(super) fn spawn(
    registry: &Arc<dyn ManagedBatchRegistry>,
    request: BatchRegistration,
) -> Result<(), VertexBatchError> {
    let runtime = Handle::try_current().map_err(|source| VertexBatchError::NoRuntime { source })?;
    let unified_object_id = request.unified_object_id.clone();
    let model_object_id = request.model_object_id.clone();
    let is_batch_create = request.create_if_missing;
    let finished = runtime.spawn(registry.store_unified_object_id(request));
    runtime.spawn(async move {
        log_batch_registration_result(
            finished.await,
            "Vertex AI",
            &unified_object_id,
            &model_object_id,
            is_batch_create,
        );
    });
    Ok(())
}
