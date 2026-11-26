use betterlite_auth::user_roles::UserRole;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::batches::records::BatchObject;
use rust_decimal::Decimal;
use secrecy::SecretString;

use super::error::AnthropicBatchError;
use super::{
    AnthropicBatchLogging, BatchRegistrant, BatchRegistration, DEFAULT_USER_ID, FILE_PURPOSE,
};
use crate::pass_through_logging::batch_attribution::{
    optional_str, request_tags_from_metadata, spawn_batch_registration,
};

pub(super) fn store(
    logging: &AnthropicBatchLogging,
    unified_object_id: &str,
    batch_object: BatchObject,
    model_object_id: &str,
    request_metadata: &JsonObject,
) -> Result<(), AnthropicBatchError> {
    let Some(registry) = logging.registry.as_ref() else {
        tracing::warn!(
            "Managed files hook not available, cannot store batch object for cost tracking"
        );
        return Ok(());
    };
    let registration = BatchRegistration {
        unified_object_id: unified_object_id.to_owned(),
        batch_object,
        model_object_id: model_object_id.to_owned(),
        file_purpose: FILE_PURPOSE.to_owned(),
        registrant: registrant(request_metadata),
        request_tags: request_tags_from_metadata(request_metadata),
        persist_attribution: true,
        create_if_missing: true,
    };
    let _handle =
        spawn_batch_registration(&logging.background, registry, "Anthropic", registration)
            .map_err(|source| AnthropicBatchError::NoRuntime { source })?;
    Ok(())
}

fn registrant(request_metadata: &JsonObject) -> BatchRegistrant {
    BatchRegistrant {
        user_id: user_id_from(request_metadata),
        api_key: request_metadata
            .get("user_api_key")
            .and_then(optional_str)
            .map(|text| SecretString::from(text.to_owned())),
        team_id: request_metadata
            .get("user_api_key_team_id")
            .and_then(optional_str)
            .map(str::to_owned),
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
    }
}

fn user_id_from(request_metadata: &JsonObject) -> Option<String> {
    request_metadata.get("user_api_key_user_id").map_or_else(
        || Some(DEFAULT_USER_ID.to_owned()),
        |value| optional_str(value).map(str::to_owned),
    )
}
