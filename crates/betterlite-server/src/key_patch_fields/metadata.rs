use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::identity_rows::Payload;
use betterlite_logging::callback_config::FeatureFlags;
use betterlite_secrets::callback_vars::{Transform, transform_callback_vars};
use betterlite_secrets::secret_codec::diagnostics::SecretDiagnostics;
use betterlite_secrets::secret_codec::{NonceSource, StoredCipher};

use crate::key_access::logging_metadata::check_key_logging_config;
use crate::management_guards::metadata::{METADATA_FIELDS, PREMIUM_METADATA_FIELDS};

use super::RESERVED_METADATA_FIELDS;
use super::Row;
use super::error::KeyPatchError;

// A category in the projection loop that ended early; the source logs it and keeps the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchWarning {
    MetadataNotAnObject,
}

impl PatchWarning {
    #[must_use]
    pub const fn subject(self) -> &'static str {
        match self {
            Self::MetadataNotAnObject => "metadata",
        }
    }
}

pub struct MetadataContext<'cipher, N, D> {
    pub flags: FeatureFlags,
    pub cipher: &'cipher StoredCipher<N, D>,
    pub warnings: &'cipher mut Vec<PatchWarning>,
}

// prepare_metadata_fields: merge the request's catalog fields into the key's metadata and encrypt it.
pub fn prepare_metadata_fields<N, D>(
    request: &Row,
    values: &mut Row,
    existing_metadata: &Row,
    context: &mut MetadataContext<'_, N, D>,
) -> Result<(), KeyPatchError>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    check_logging_config(values.get("metadata"), context.flags)?;
    if !values.contains_key("metadata") {
        values.insert(
            String::from("metadata"),
            JsonData::Object(existing_metadata.clone()),
        );
    }
    let mut casted = object_or_null(values.get("metadata"));
    apply_reserved_fields(&mut casted, existing_metadata)?;
    project_catalog_fields(request, &mut casted, context.warnings);
    let metadata = encrypt_metadata(casted, context.cipher)?;
    values.insert(String::from("metadata"), metadata);
    Ok(())
}

// The logging validator runs first, on the request's metadata, before any copy or reserved-field check.
fn check_logging_config(
    metadata: Option<&JsonData>,
    flags: FeatureFlags,
) -> Result<(), KeyPatchError> {
    let Some(JsonData::Object(fields)) = metadata else {
        return Ok(());
    };
    check_key_logging_config(Some(&Payload(fields.clone())), flags)
        .map_err(|error| KeyPatchError::LoggingMetadata(Box::new(error)))
}

fn object_or_null(value: Option<&JsonData>) -> Option<BTreeMap<String, JsonData>> {
    match value {
        Some(JsonData::Object(fields)) => Some(fields.clone()),
        Some(
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::String(_)
            | JsonData::Array(_),
        )
        | None => None,
    }
}

// A reserved field keeps the stored value; a null request metadata or a different value is the 400.
fn apply_reserved_fields(
    casted: &mut Option<BTreeMap<String, JsonData>>,
    existing: &Row,
) -> Result<(), KeyPatchError> {
    for field in RESERVED_METADATA_FIELDS {
        let Some(stored) = existing
            .get(field)
            .filter(|value| !matches!(value, JsonData::Null))
        else {
            continue;
        };
        let changed = casted
            .as_ref()
            .is_none_or(|fields| fields.get(field).is_some_and(|value| value != stored));
        if changed {
            return Err(KeyPatchError::ImmutableMetadata {
                field: field.to_owned(),
            });
        }
        if let Some(fields) = casted.as_mut() {
            fields.insert(field.to_owned(), stored.clone());
        }
    }
    Ok(())
}

// The source writes each present catalog field into the metadata; with no metadata map the first one raises.
fn project_catalog_fields(
    request: &Row,
    casted: &mut Option<BTreeMap<String, JsonData>>,
    warnings: &mut Vec<PatchWarning>,
) {
    let Some(fields) = casted.as_mut() else {
        if request.keys().any(|field| is_catalog_field(field)) {
            warnings.push(PatchWarning::MetadataNotAnObject);
        }
        return;
    };
    for field in PREMIUM_METADATA_FIELDS {
        if let Some(value) = request.get(field) {
            fields.insert(field.to_owned(), value.clone());
        }
    }
    for (field, value) in request {
        if is_catalog_field(field) {
            fields.insert(field.clone(), value.clone());
        }
    }
}

#[must_use]
// ast-grep-ignore: no-one-line-functions - one catalog predicate shared by the projection loop and the key-field filter
pub(crate) fn is_catalog_field(field: &str) -> bool {
    METADATA_FIELDS.contains(&field) || PREMIUM_METADATA_FIELDS.contains(&field)
}

fn encrypt_metadata<N, D>(
    casted: Option<BTreeMap<String, JsonData>>,
    cipher: &StoredCipher<N, D>,
) -> Result<JsonData, KeyPatchError>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    let Some(fields) = casted else {
        return Ok(JsonData::Null);
    };
    let value = serde_json::to_value(JsonData::Object(fields))?;
    let encrypted = transform_callback_vars(&value, &Transform::Encrypt(cipher));
    Ok(serde_json::from_value(encrypted)?)
}

const AWARE_SECONDS: &str = "%Y-%m-%dT%H:%M:%S%:z";
const AWARE_MICROS: &str = "%Y-%m-%dT%H:%M:%S%.6f%:z";
const NAIVE_SECONDS: &str = "%Y-%m-%dT%H:%M:%S";
const NAIVE_MICROS: &str = "%Y-%m-%dT%H:%M:%S%.6f";

// pydantic hands the loop a datetime, so its metadata copy is isoformat(): own offset, micros only when nonzero.
#[must_use]
pub fn python_isoformat(value: &Timestamp) -> String {
    match value {
        Timestamp::Aware(aware) => {
            let format = if aware.naive_local().and_utc().timestamp_subsec_micros() != 0 {
                AWARE_MICROS
            } else {
                AWARE_SECONDS
            };
            aware.format(format).to_string()
        }
        Timestamp::Naive(naive) => {
            let format = if naive.and_utc().timestamp_subsec_micros() != 0 {
                NAIVE_MICROS
            } else {
                NAIVE_SECONDS
            };
            naive.format(format).to_string()
        }
    }
}
