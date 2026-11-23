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
use super::object_of;

// A category in the projection loop that ended early; the source logs it and keeps the request.
pub type PatchWarning = &'static str;
pub const METADATA_WARNING: PatchWarning = "metadata";

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
    if let Some(JsonData::Object(fields)) = values.get("metadata") {
        check_key_logging_config(Some(&Payload(fields.clone())), context.flags)
            .map_err(|error| KeyPatchError::LoggingMetadata(Box::new(error)))?;
    }
    if !values.contains_key("metadata") {
        values.insert(
            String::from("metadata"),
            JsonData::Object(existing_metadata.clone()),
        );
    }
    let mut casted = values.get("metadata").and_then(object_of).cloned();
    for field in RESERVED_METADATA_FIELDS {
        let Some(stored) = existing_metadata
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
    match casted.as_mut() {
        Some(fields) => {
            for (field, value) in request {
                if is_catalog_field(field) {
                    fields.insert(field.clone(), value.clone());
                }
            }
        }
        None => {
            if request.keys().any(|field| is_catalog_field(field)) {
                context.warnings.push(METADATA_WARNING);
            }
        }
    }
    let metadata = match casted {
        Some(fields) => {
            let value = serde_json::to_value(JsonData::Object(fields))?;
            let encrypted = transform_callback_vars(&value, &Transform::Encrypt(context.cipher));
            serde_json::from_value(encrypted)?
        }
        None => JsonData::Null,
    };
    values.insert(String::from("metadata"), metadata);
    Ok(())
}

#[must_use]
// ast-grep-ignore: no-one-line-functions - one catalog predicate shared by the projection loop and the key-field filter
pub(crate) fn is_catalog_field(field: &str) -> bool {
    METADATA_FIELDS.contains(&field) || PREMIUM_METADATA_FIELDS.contains(&field)
}

const AWARE_SECONDS: &str = "%Y-%m-%dT%H:%M:%S%:z";
const AWARE_MICROS: &str = "%Y-%m-%dT%H:%M:%S%.6f%:z";
const NAIVE_SECONDS: &str = "%Y-%m-%dT%H:%M:%S";
const NAIVE_MICROS: &str = "%Y-%m-%dT%H:%M:%S%.6f";

// pydantic hands the loop a datetime, so its metadata copy is isoformat(): own offset, micros only when nonzero.
#[must_use]
pub fn isoformat_timestamp(value: &Timestamp) -> String {
    match value {
        Timestamp::Aware(aware) => {
            let micros = aware.naive_local().and_utc().timestamp_subsec_micros();
            aware
                .format(if micros == 0 {
                    AWARE_SECONDS
                } else {
                    AWARE_MICROS
                })
                .to_string()
        }
        Timestamp::Naive(naive) => {
            let micros = naive.and_utc().timestamp_subsec_micros();
            naive
                .format(if micros == 0 {
                    NAIVE_SECONDS
                } else {
                    NAIVE_MICROS
                })
                .to_string()
        }
    }
}
