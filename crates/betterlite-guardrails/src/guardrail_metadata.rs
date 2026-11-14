use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_records::record::GuardrailInvocationRecord;
use crate::openai_moderation::logging::{GuardrailLogRecord, GuardrailResponse};
use crate::panw_prisma_airs::audit::{GuardrailAudit, Stage};
use entry::scan_entry_value;

pub const SCAN_IDS_KEY: &str = "guardrail_scan_ids";
pub const SCAN_METADATA_KEY: &str = "guardrail_scan_metadata";
pub const MAX_SCAN_METADATA_LENGTH: usize = 2048;

const APPLIED_GUARDRAILS_KEY: &str = "applied_guardrails";
const APPLIED_POLICIES_KEY: &str = "applied_policies";
const GUARDRAIL_INFORMATION_KEY: &str = "guardrail_information";
const OPENAI_MODERATION_PROVIDER: &str = "openai_moderation";
const PILLAR_FLAGGED_KEY: &str = "pillar_flagged";
const PILLAR_RESPONSE_HEADERS_KEY: &str = "pillar_response_headers";
const POLICY_SOURCES_KEY: &str = "policy_sources";
const SEMANTIC_SIMILARITY_KEY: &str = "semantic-similarity";
const STANDARD_LOGGING_OBJECT_KEY: &str = "standard_logging_object";
const TRUSTED_PILLAR_HEADERS_KEY: &str = "_pillar_response_headers_trusted";

pub type RequestData = BTreeMap<String, GuardrailValue>;

// LiteLLM's GuardrailScanMetadata, stored in guardrail_scan_metadata and serialized into its header
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanEntry {
    pub guardrail: Option<String>,
    pub stage: GuardrailEventHooks,
    pub provider: String,
    pub scan_id: String,
}

#[derive(Debug, Error)]
pub enum GuardrailMetadataError {
    #[error("the request metadata field {field} must be {expected}")]
    NotTyped {
        field: &'static str,
        expected: &'static str,
    },
    #[error("guardrail scan metadata entry {index} is not a scan entry")]
    ScanEntry {
        index: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("the guardrail invocation record is not JSON")]
    Record {
        #[source]
        source: serde_json::Error,
    },
}

fn append_unique(bucket: &mut BTreeMap<String, GuardrailValue>, key: &str, value: GuardrailValue) {
    let mut values = match bucket.remove(key) {
        Some(GuardrailValue::Array(values)) => values,
        None | Some(_) => Vec::new(),
    };
    if !values.contains(&value) {
        values.push(value);
    }
    bucket.insert(key.to_owned(), GuardrailValue::Array(values));
}

// LiteLLM's add_guardrail_scan_id: one id list and one complete-entry list, deduplicated separately
pub fn record_scan_id(
    request_data: &mut RequestData,
    scan_id: Option<&str>,
    guardrail: Option<&str>,
    provider: &str,
    stage: GuardrailEventHooks,
) {
    let Some(id) = scan_id.filter(|found| !found.is_empty()) else {
        return;
    };
    // LiteLLM's get_or_create_metadata_bucket: a bucket that is absent or not an object becomes one
    let key = if request_data.contains_key("litellm_metadata") {
        "litellm_metadata"
    } else {
        "metadata"
    };
    let mut bucket = match request_data.remove(key) {
        Some(GuardrailValue::Object(fields)) => fields,
        None
        | Some(
            GuardrailValue::Null
            | GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Text(_)
            | GuardrailValue::Array(_),
        ) => BTreeMap::new(),
    };
    append_unique(
        &mut bucket,
        SCAN_IDS_KEY,
        GuardrailValue::Text(id.to_owned()),
    );
    append_unique(
        &mut bucket,
        SCAN_METADATA_KEY,
        scan_entry_value(guardrail, provider, stage, id),
    );
    request_data.insert(key.to_owned(), GuardrailValue::Object(bucket));
}

// LiteLLM's _serialize_scan_metadata_header, whose length counts Python str characters
pub fn serialize_scan_metadata_header(
    entries: &[GuardrailValue],
    max_length: usize,
) -> Result<Option<String>, GuardrailMetadataError> {
    let mut encoded = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let scan_entry: ScanEntry = serde_json::to_value(entry)
            .and_then(serde_json::from_value)
            .map_err(|source| GuardrailMetadataError::ScanEntry { index, source })?;
        let text = serde_json::to_string(&scan_entry)
            .map_err(|source| GuardrailMetadataError::ScanEntry { index, source })?;
        encoded.push(text);
    }
    let mut total = 0_usize;
    let mut kept = 0_usize;
    for item in &encoded {
        let Some(with_separator) = total
            .checked_add(item.chars().count())
            .and_then(|sum| sum.checked_add(1))
        else {
            break;
        };
        total = with_separator;
        let Some(header_length) = total.checked_add(1) else {
            break;
        };
        if header_length > max_length {
            break;
        }
        kept = kept.saturating_add(1);
    }
    if kept == 0 {
        return Ok(None);
    }
    let kept_items: Vec<&str> = encoded.iter().take(kept).map(String::as_str).collect();
    Ok(Some(format!("[{}]", kept_items.join(","))))
}

// LiteLLM's get_logging_caching_headers: metadata first, then litellm_metadata over it by key
pub fn logging_caching_headers(
    request_data: &RequestData,
) -> Result<Vec<(String, String)>, GuardrailMetadataError> {
    let merged = projection::merged_metadata(request_data);
    let mut headers = Vec::new();
    projection::push_optional(
        &mut headers,
        projection::joined_header(
            &merged,
            APPLIED_GUARDRAILS_KEY,
            "x-litellm-applied-guardrails",
            projection::EmptyList::Emit,
        )?,
    );
    projection::push_optional(
        &mut headers,
        projection::joined_header(
            &merged,
            SCAN_IDS_KEY,
            "x-litellm-guardrail-scan-id",
            projection::EmptyList::Omit,
        )?,
    );
    projection::push_optional(&mut headers, projection::scan_metadata_header(&merged)?);
    projection::push_optional(
        &mut headers,
        projection::joined_header(
            &merged,
            APPLIED_POLICIES_KEY,
            "x-litellm-applied-policies",
            projection::EmptyList::Emit,
        )?,
    );
    projection::push_optional(&mut headers, projection::policy_sources_header(&merged)?);
    projection::push_optional(
        &mut headers,
        projection::semantic_similarity_header(&merged)?,
    );
    headers.extend(projection::pillar_headers(&merged)?);
    Ok(headers)
}

fn standard_logging_object(
    details: &mut BTreeMap<String, GuardrailValue>,
) -> Result<Option<&mut BTreeMap<String, GuardrailValue>>, GuardrailMetadataError> {
    match details.get_mut(STANDARD_LOGGING_OBJECT_KEY) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Object(fields)) => Ok(Some(fields)),
        Some(
            GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Text(_)
            | GuardrailValue::Array(_),
        ) => Err(GuardrailMetadataError::NotTyped {
            field: STANDARD_LOGGING_OBJECT_KEY,
            expected: "an object",
        }),
    }
}

// LiteLLM's add_guardrail_response_to_standard_logging_object: a missing logging object stays missing
pub fn append_guardrail_response(
    model_call_details: Option<&mut BTreeMap<String, GuardrailValue>>,
    record: &GuardrailInvocationRecord,
) -> Result<(), GuardrailMetadataError> {
    let Some(details) = model_call_details else {
        return Ok(());
    };
    let Some(standard) = standard_logging_object(details)? else {
        return Ok(());
    };
    let mut entries = match standard.get(GUARDRAIL_INFORMATION_KEY) {
        None | Some(GuardrailValue::Null) => Vec::new(),
        Some(GuardrailValue::Array(entries)) => entries.clone(),
        Some(_) => {
            return Err(GuardrailMetadataError::NotTyped {
                field: GUARDRAIL_INFORMATION_KEY,
                expected: "a list",
            });
        }
    };
    let entry = serde_json::to_value(record)
        .and_then(serde_json::from_value)
        .map_err(|source| GuardrailMetadataError::Record { source })?;
    entries.push(entry);
    standard.insert(
        GUARDRAIL_INFORMATION_KEY.to_owned(),
        GuardrailValue::Array(entries),
    );
    Ok(())
}

// LiteLLM's PANW hook records each parsed scan id as its stage runs
pub fn merge_prisma_audit(request_data: &mut RequestData, audit: &GuardrailAudit) {
    for entry in &audit.scan_metadata {
        let stage = match entry.stage {
            Stage::PreCall => GuardrailEventHooks::PreCall,
            Stage::PostCall => GuardrailEventHooks::PostCall,
        };
        record_scan_id(
            request_data,
            Some(&entry.scan_id),
            Some(&entry.guardrail),
            &entry.provider,
            stage,
        );
    }
}

// LiteLLM's moderation hook records the id of the response it just consumed, before it blocks
pub fn merge_moderation_scan(
    request_data: &mut RequestData,
    records: &[GuardrailLogRecord],
    start_index: usize,
    guardrail_name: Option<&str>,
    input_type: GuardrailInputType,
) {
    let stage = match input_type {
        GuardrailInputType::Request => GuardrailEventHooks::PreCall,
        GuardrailInputType::Response => GuardrailEventHooks::PostCall,
    };
    for record in records.iter().skip(start_index) {
        let GuardrailResponse::Moderation(response) = &record.guardrail_response else {
            continue;
        };
        record_scan_id(
            request_data,
            Some(&response.id),
            guardrail_name,
            OPENAI_MODERATION_PROVIDER,
            stage,
        );
    }
}

mod entry;
mod projection;
mod tests;
