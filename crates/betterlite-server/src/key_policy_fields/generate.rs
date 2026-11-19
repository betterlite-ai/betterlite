use std::collections::BTreeMap;

use betterlite_core::budget_reset::compute_budget_reset_at;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::search_tools::convert::isoformat;
use betterlite_db::team_rows::member::BudgetLimitEntry;
use betterlite_secrets::callback_vars::{Transform, transform_callback_vars};
use betterlite_secrets::secret_codec::diagnostics::SecretDiagnostics;
use betterlite_secrets::secret_codec::{NonceSource, StoredCipher};

use crate::key_fields::decode::{DecodeError, to_object};
use crate::key_fields::request::GenerateKeyRequest;
use crate::key_fields::values::LiteLLMKeyType;
use crate::key_policy_fields::columns::{ColumnError, verification_token_from_row};
use crate::key_policy_fields::key_type::handle_key_type;
use crate::key_policy_fields::{
    ClockError, KeyClock, microsecond_precision, shifted_by, zulu_datetime,
};
use crate::management_guards::metadata::{METADATA_FIELDS, PREMIUM_METADATA_FIELDS};

#[derive(Debug, thiserror::Error)]
pub enum GenerateError {
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error(transparent)]
    Column(#[from] ColumnError),
    #[error(transparent)]
    Clock(#[from] ClockError),
}

// the effective key a generate request would store: supplied fields only, metadata folded and encrypted
pub fn effective_key_for_generate<N, D>(
    request: &GenerateKeyRequest,
    clock: KeyClock<'_>,
    cipher: &StoredCipher<N, D>,
) -> Result<LiteLLMVerificationToken, GenerateError>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    let mut requested = request.project()?.supplied;
    requested.retain(|_field, value| !matches!(value, JsonData::Null));
    let mut metadata_fields = BTreeMap::new();
    let mut supplied_columns = BTreeMap::new();
    for (field, value) in requested {
        if PREMIUM_METADATA_FIELDS.contains(&field.as_str())
            || METADATA_FIELDS.contains(&field.as_str())
        {
            metadata_fields.insert(field, value);
        } else {
            supplied_columns.insert(field, value);
        }
    }
    let mut row = handle_key_type(resolved_key_type(request), supplied_columns);
    let base = &request.base.base;
    let mut metadata = base.metadata.clone().flatten().unwrap_or_default().0;
    metadata.extend(metadata_fields);
    row.insert("metadata".to_owned(), encrypt_metadata(metadata, cipher)?);
    insert_timing_fields(&mut row, request, clock)?;
    row.insert("object_permission".to_owned(), JsonData::Null);
    Ok(verification_token_from_row(&row)?)
}

// the timing columns a generate request stores: expiry, budget reset, rotation and the budget windows
fn insert_timing_fields(
    row: &mut BTreeMap<String, JsonData>,
    request: &GenerateKeyRequest,
    clock: KeyClock<'_>,
) -> Result<(), GenerateError> {
    let base = &request.base.base;
    let duration = base.duration.as_ref().and_then(Option::as_ref);
    let budget_duration = base.budget_duration.as_ref().and_then(Option::as_ref);
    let windows = generate_budget_windows(base.budget_limits.clone().flatten().as_deref(), clock)?;
    let expires = match duration {
        Some(spec) => JsonData::String(zulu_datetime(shifted_by(clock.now, spec, clock)?)),
        None => JsonData::Null,
    };
    let reset = match budget_duration {
        Some(spec) => {
            let at = compute_budget_reset_at(spec, clock.reset, clock.now, clock.database)
                .map_err(|source| ClockError::Duration {
                    spec: spec.to_owned(),
                    source,
                })?;
            JsonData::String(zulu_datetime(microsecond_precision(at.instant)))
        }
        None => JsonData::Null,
    };
    let rotation_at = rotation_field(request, clock)?;
    row.insert("expires".to_owned(), expires);
    row.insert("budget_reset_at".to_owned(), reset);
    row.insert("key_rotation_at".to_owned(), rotation_at);
    let limits = windows.map_or(JsonData::Null, JsonData::Array);
    row.insert("budget_limits".to_owned(), limits);
    Ok(())
}

// the pydantic model defaults key_type to DEFAULT, so only an explicit null leaves the column out
#[must_use]
pub fn resolved_key_type(request: &GenerateKeyRequest) -> Option<LiteLLMKeyType> {
    request
        .generate
        .key_type
        .unwrap_or(Some(LiteLLMKeyType::Default))
        .flatten()
}

// an absent, null or empty window list stores no windows; each kept window keeps its fields and a fresh reset time
pub fn generate_budget_windows(
    windows: Option<&[BudgetLimitEntry]>,
    clock: KeyClock<'_>,
) -> Result<Option<Vec<JsonData>>, GenerateError> {
    let Some(entries) = windows else {
        return Ok(None);
    };
    if entries.is_empty() {
        return Ok(None);
    }
    let mut built = Vec::with_capacity(entries.len());
    for window in entries {
        let mut fields = to_object(window)?;
        let reset = compute_budget_reset_at(
            &window.budget_duration,
            clock.reset,
            clock.now,
            clock.database,
        )
        .map_err(|source| ClockError::Duration {
            spec: window.budget_duration.clone(),
            source,
        })?;
        let reset_at = isoformat(microsecond_precision(reset.instant));
        fields.insert("reset_at".to_owned(), JsonData::String(reset_at));
        built.push(JsonData::Object(fields));
    }
    Ok(Some(built))
}

// the two slots that hold callback vars are encrypted with the proxy's own cipher before they are stored
fn encrypt_metadata<N, D>(
    metadata: BTreeMap<String, JsonData>,
    cipher: &StoredCipher<N, D>,
) -> Result<JsonData, DecodeError>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    let value = serde_json::to_value(JsonData::Object(metadata)).map_err(DecodeError::Document)?;
    let encrypted = transform_callback_vars(&value, &Transform::Encrypt(cipher));
    serde_json::from_value(encrypted).map_err(DecodeError::Document)
}

fn rotation_field(
    request: &GenerateKeyRequest,
    clock: KeyClock<'_>,
) -> Result<JsonData, GenerateError> {
    let rotate = request.generate.auto_rotate.flatten().unwrap_or(false);
    if !rotate {
        return Ok(JsonData::Null);
    }
    let interval = request
        .generate
        .rotation_interval
        .as_ref()
        .and_then(Option::as_ref)
        .map(String::as_str);
    let Some(spec) = interval.filter(|text| !text.is_empty()) else {
        return Ok(JsonData::Null);
    };
    let instant = shifted_by(clock.now, spec, clock)?;
    Ok(JsonData::String(zulu_datetime(instant)))
}
