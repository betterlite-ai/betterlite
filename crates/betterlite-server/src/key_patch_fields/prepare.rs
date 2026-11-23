mod timing;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::resource_repository::model::ModelRepository;
use betterlite_logging::callback_config::FeatureFlags;
use betterlite_models::decisions::Json;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_secrets::secret_codec::diagnostics::SecretDiagnostics;
use betterlite_secrets::secret_codec::{NonceSource, StoredCipher};

use crate::key_fields::update::UpdateKeyRequest;
use crate::key_policy_fields::KeyClock;
use crate::key_policy_fields::model_budget::validate_model_max_budget;
use crate::router_weight_references::settings::validate_settings_weights;

use self::timing::{apply_budget_windows, apply_request_times};
use super::Row;
use super::drop_request_only_fields;
use super::error::KeyPatchError;
use super::metadata::{
    MetadataContext, PatchWarning, is_catalog_field, isoformat_timestamp, prepare_metadata_fields,
};
use super::object_of;
use super::string_of;

pub struct PrepareContext<'existing> {
    pub existing: &'existing LiteLLMVerificationToken,
    pub clock: KeyClock<'existing>,
    pub flags: FeatureFlags,
    pub models: Option<&'existing ModelRepository<'existing>>,
    pub configured: Option<&'existing DeploymentCatalog>,
    pub warnings: &'existing mut Vec<PatchWarning>,
}

// prepare_key_update_data: the patch a key update would write, with metadata folded and router weights checked.
pub async fn prepare_key_update_data<N, D>(
    request: &UpdateKeyRequest,
    cipher: &StoredCipher<N, D>,
    context: &mut PrepareContext<'_>,
) -> Result<Row, KeyPatchError>
where
    N: NonceSource + Sync,
    D: SecretDiagnostics + Sync,
{
    let mut supplied = request.project()?.supplied;
    validate_router_settings(&supplied, context).await?;
    check_service_account_team(&supplied, context.existing.team_id.as_deref())?;
    let fields = metadata_fields(request, supplied.clone());
    drop_request_only_fields(&mut supplied);
    let mut values = key_fields_only(&supplied);
    apply_request_times(&mut values, context.clock)?;
    apply_budget_windows(&mut values, context.clock)?;
    validate_model_max_budget(values.get("model_max_budget").and_then(object_of))?;
    serialize_router_settings(&mut values)?;
    let mut metadata_context = MetadataContext {
        flags: context.flags,
        cipher,
        warnings: &mut *context.warnings,
    };
    prepare_metadata_fields(
        &fields,
        &mut values,
        &context.existing.metadata.0,
        &mut metadata_context,
    )?;
    Ok(values)
}

// the pydantic dump prepare_metadata_fields folds, without nulls, with each datetime rendered as isoformat
#[must_use]
pub fn metadata_fields(request: &UpdateKeyRequest, supplied: Row) -> Row {
    let expiry = request
        .update
        .temp_budget_expiry
        .as_ref()
        .and_then(Option::as_ref);
    let mut fields: Row = supplied
        .into_iter()
        .filter(|(_, value)| !matches!(value, JsonData::Null))
        .collect();
    if let Some(timestamp) = expiry {
        fields.insert(
            String::from("temp_budget_expiry"),
            JsonData::String(isoformat_timestamp(timestamp)),
        );
    }
    fields
}

// The request's own fields, without the two metadata catalogs.
fn key_fields_only(supplied: &Row) -> Row {
    let mut values = Row::new();
    for (name, value) in supplied {
        if !is_catalog_field(name) {
            values.insert(name.clone(), value.clone());
        }
    }
    values
}

fn check_service_account_team(
    supplied: &Row,
    existing_team: Option<&str>,
) -> Result<(), KeyPatchError> {
    let Some(JsonData::Object(metadata)) = supplied.get("metadata") else {
        return Ok(());
    };
    let account = metadata.get("service_account_id");
    if account.is_none_or(|value| matches!(value, JsonData::Null)) {
        return Ok(());
    }
    if supplied
        .get("team_id")
        .and_then(string_of)
        .or(existing_team)
        .is_some()
    {
        return Ok(());
    }
    Err(KeyPatchError::ServiceAccountTeamRequired)
}

// The source validates router weights when they are supplied or when an omitted patch inherits a new team's.
async fn validate_router_settings(
    supplied: &Row,
    context: &PrepareContext<'_>,
) -> Result<(), KeyPatchError> {
    let router_settings = supplied.get("router_settings");
    let explicit = matches!(router_settings, Some(value) if !matches!(value, JsonData::Null));
    let team_changed = supplied.contains_key("team_id")
        && supplied.get("team_id").and_then(string_of) != context.existing.team_id.as_deref();
    let needs_validation = explicit || (router_settings.is_none() && team_changed);
    if !needs_validation {
        return Ok(());
    }
    let effective = match router_settings {
        Some(JsonData::Null) | None => stored_router_settings(context.existing)?,
        Some(value) => Some(serde_json::from_value(serde_json::to_value(value)?)?),
    };
    let team_id = if supplied.contains_key("team_id") {
        supplied.get("team_id").and_then(string_of)
    } else {
        context.existing.team_id.as_deref()
    };
    validate_settings_weights(
        effective.as_ref(),
        team_id,
        context.models,
        context.configured,
    )
    .await?;
    Ok(())
}

// A stored router_settings column is decoded with the row, so only the Json conversion is left.
fn stored_router_settings(
    existing: &LiteLLMVerificationToken,
) -> Result<Option<Json>, KeyPatchError> {
    let Some(settings) = &existing.router_settings else {
        return Ok(None);
    };
    let value = serde_json::to_value(settings)?;
    Ok(Some(serde_json::from_value(value)?))
}

// safe_dumps of a nonnull router_settings: the stored column is JSON text, not an object
fn serialize_router_settings(values: &mut Row) -> Result<(), KeyPatchError> {
    let Some(value) = values.get("router_settings") else {
        return Ok(());
    };
    if !matches!(value, JsonData::Null) {
        let text = serde_json::to_string(value)?;
        values.insert(String::from("router_settings"), JsonData::String(text));
    }
    Ok(())
}
