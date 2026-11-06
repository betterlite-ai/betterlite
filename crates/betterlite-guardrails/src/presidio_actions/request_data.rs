use betterlite_config::sections::guardrail_identity::enums::PiiEntityType;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::value::RawValue;

use crate::presidio_actions::error::PresidioActionsError;

pub type RequestData = IndexMap<String, Box<RawValue>>;

const LOGGING_KEY: &str = "standard_logging_guardrail_information";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresidioPerRequestConfig {
    pub language: Option<String>,
    pub entities: Option<Vec<PiiEntityType>>,
}

pub fn presidio_settings_from_request(
    request_data: &RequestData,
) -> Result<Option<PresidioPerRequestConfig>, PresidioActionsError> {
    let Some(metadata) = metadata_object(request_data, "metadata")? else {
        return Ok(None);
    };
    let Some(config_raw) = metadata.get("guardrail_config") else {
        return Ok(None);
    };
    let parsed = serde_json::from_str::<Option<RequestData>>(config_raw.get())
        .map_err(|source| PresidioActionsError::GuardrailConfigNotObject { source })?;
    let Some(fields) = parsed else {
        return Ok(None);
    };
    // LiteLLM only builds a config from a non-empty guardrail_config dict.
    if fields.is_empty() {
        return Ok(None);
    }
    let language = fields
        .get("language")
        .map(|value_raw| serde_json::from_str::<Option<String>>(value_raw.get()))
        .transpose()
        .map_err(|source| PresidioActionsError::GuardrailConfigField {
            field: "language",
            source,
        })?
        .flatten();
    let entities = fields
        .get("entities")
        .map(|value_raw| serde_json::from_str::<Option<Vec<PiiEntityType>>>(value_raw.get()))
        .transpose()
        .map_err(|source| PresidioActionsError::GuardrailConfigField {
            field: "entities",
            source,
        })?
        .flatten();
    Ok(Some(PresidioPerRequestConfig { language, entities }))
}

pub fn pii_tokens(
    request_data: &RequestData,
) -> Result<IndexMap<String, String>, PresidioActionsError> {
    let Some(metadata) = metadata_object(request_data, "metadata")? else {
        return Ok(IndexMap::new());
    };
    let Some(raw) = metadata.get("pii_tokens") else {
        return Ok(IndexMap::new());
    };
    serde_json::from_str::<Option<IndexMap<String, String>>>(raw.get())
        .map(Option::unwrap_or_default)
        .map_err(|source| PresidioActionsError::PiiTokensNotObject { source })
}

pub(super) fn store_pii_tokens(
    request_data: &mut RequestData,
    tokens: &IndexMap<String, String>,
) -> Result<(), PresidioActionsError> {
    let mut metadata = metadata_object(request_data, "metadata")?.unwrap_or_default();
    let mut stored = metadata.get("pii_tokens").map_or_else(
        || Ok(IndexMap::new()),
        |raw| {
            serde_json::from_str::<IndexMap<String, String>>(raw.get())
                .map_err(|source| PresidioActionsError::PiiTokensNotObject { source })
        },
    )?;
    for (token, original) in tokens {
        stored.insert(token.clone(), original.clone());
    }
    metadata.insert("pii_tokens".to_owned(), raw_value(&stored)?);
    store_metadata(request_data, "metadata", &metadata)
}

pub(super) fn append_guardrail_information<Record>(
    request_data: &mut RequestData,
    record: &Record,
) -> Result<(), PresidioActionsError>
where
    Record: Serialize,
{
    let bucket = if request_data.contains_key("litellm_metadata") {
        "litellm_metadata"
    } else {
        "metadata"
    };
    // LiteLLM replaces a metadata bucket that is not a dict with a fresh one.
    let mut metadata = match metadata_object(request_data, bucket) {
        Ok(Some(metadata)) => metadata,
        Ok(None) | Err(_) => RequestData::new(),
    };
    let mut entries = metadata.get(LOGGING_KEY).map_or_else(Vec::new, |raw| {
        serde_json::from_str::<Vec<Box<RawValue>>>(raw.get())
            .unwrap_or_else(|_error| vec![raw.clone()])
    });
    entries.push(raw_value(record)?);
    metadata.insert(LOGGING_KEY.to_owned(), raw_value(&entries)?);
    store_metadata(request_data, bucket, &metadata)
}

fn raw_value<Value>(value: &Value) -> Result<Box<RawValue>, PresidioActionsError>
where
    Value: Serialize,
{
    let encoded = serde_json::to_string(value)
        .map_err(|source| PresidioActionsError::Serialize { source })?;
    RawValue::from_string(encoded).map_err(|source| PresidioActionsError::Serialize { source })
}

fn metadata_object(
    request_data: &RequestData,
    key: &str,
) -> Result<Option<RequestData>, PresidioActionsError> {
    let Some(raw) = request_data.get(key) else {
        return Ok(None);
    };
    serde_json::from_str::<Option<RequestData>>(raw.get())
        .map_err(|source| PresidioActionsError::MetadataNotObject { source })
}

fn store_metadata(
    request_data: &mut RequestData,
    bucket: &str,
    metadata: &RequestData,
) -> Result<(), PresidioActionsError> {
    let raw = raw_value(metadata)?;
    request_data.insert(bucket.to_owned(), raw);
    Ok(())
}
