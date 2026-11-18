use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::{
    GUARDRAIL_PIPELINES, PIPELINE_MANAGED_GUARDRAILS, PolicyPipelines, RESPONSE_ID,
    ResponseRetrievalError, RetrievalData,
};
use crate::pipeline::json_value::JsonValue;
use crate::resolver::pipelines::pipeline_managed_guardrails;

pub(super) const METADATA: &str = "metadata";
pub(super) const LITELLM_METADATA: &str = "litellm_metadata";
pub(super) const APPLIED_POLICIES: &str = "applied_policies";
pub(super) const APPLIED_GUARDRAILS: &str = "applied_guardrails";
pub(super) const POLICY_SOURCES: &str = "policy_sources";

#[must_use]
pub(super) fn response_id(data: &RetrievalData) -> Option<&str> {
    match data.get(RESPONSE_ID) {
        Some(JsonValue::String(id)) => Some(id.as_str()),
        None
        | Some(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_),
        ) => None,
    }
}

// the id as Python's str() prints it, for the log lines that carry it
#[must_use]
pub(super) fn response_id_text(data: &RetrievalData) -> String {
    let Some(value) = data.get(RESPONSE_ID) else {
        return "None".to_owned();
    };
    if let JsonValue::String(id) = value {
        return id.clone();
    }
    if let JsonValue::Bool(value) = value {
        return if *value {
            "True".to_owned()
        } else {
            "False".to_owned()
        };
    }
    if let JsonValue::Number(number) = value {
        return number.to_string();
    }
    format!("{value:?}")
}

#[must_use]
pub(super) fn metadata_name(data: &RetrievalData) -> &'static str {
    if matches!(data.get(LITELLM_METADATA), Some(JsonValue::Object(_))) {
        return LITELLM_METADATA;
    }
    METADATA
}

// LiteLLM's get_or_create_metadata_bucket: a bucket that is absent or not an object becomes one
pub(super) fn bucket_mut<'data>(
    data: &'data mut RetrievalData,
    name: &str,
) -> Result<&'data mut BTreeMap<String, JsonValue>, ResponseRetrievalError> {
    if !matches!(data.get(name), Some(JsonValue::Object(_))) {
        data.insert(name.to_owned(), JsonValue::Object(BTreeMap::new()));
    }
    let Some(JsonValue::Object(bucket)) = data.get_mut(name) else {
        return Err(ResponseRetrievalError::MetadataBucket);
    };
    Ok(bucket)
}

pub(super) fn decode_policy_pipelines(
    value: Option<&JsonValue>,
) -> Result<PolicyPipelines, ResponseRetrievalError> {
    match value {
        None | Some(JsonValue::Null) => Ok(Vec::new()),
        Some(JsonValue::Array(items)) => {
            let text = serde_json::to_string(items).map_err(ResponseRetrievalError::Pipelines)?;
            serde_json::from_str(&text).map_err(ResponseRetrievalError::Pipelines)
        }
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) | JsonValue::Object(_),
        ) => Err(ResponseRetrievalError::NotTyped {
            field: GUARDRAIL_PIPELINES,
            expected: "a list of policy name and pipeline pairs",
        }),
    }
}

pub(super) fn new_pipelines(
    data: &mut RetrievalData,
    pipelines: &PolicyPipelines,
) -> Result<Option<(PolicyPipelines, PolicyPipelines)>, ResponseRetrievalError> {
    let name = metadata_name(data);
    let bucket = bucket_mut(data, name)?;
    let already_attached = decode_policy_pipelines(bucket.get(GUARDRAIL_PIPELINES))?;
    let attached_names: BTreeSet<String> = already_attached
        .iter()
        .map(|(policy_name, _)| policy_name.clone())
        .collect();
    let added: PolicyPipelines = pipelines
        .iter()
        .filter(|(policy_name, _)| !attached_names.contains(policy_name))
        .cloned()
        .collect();
    if added.is_empty() {
        return Ok(None);
    }
    Ok(Some((already_attached, added)))
}

pub(super) fn attach_new_pipelines(
    data: &mut RetrievalData,
    already_attached: PolicyPipelines,
    added: &PolicyPipelines,
    sources: &BTreeMap<String, String>,
) -> Result<(), ResponseRetrievalError> {
    store_pipelines(data, already_attached, added)?;
    add_headers(data, added, sources)
}

fn store_pipelines(
    data: &mut RetrievalData,
    already_attached: PolicyPipelines,
    added: &PolicyPipelines,
) -> Result<(), ResponseRetrievalError> {
    let mut pipelines = already_attached;
    pipelines.extend(added.iter().cloned());
    let name = metadata_name(data);
    let bucket = bucket_mut(data, name)?;
    bucket.insert(
        GUARDRAIL_PIPELINES.to_owned(),
        json_from_serialize(&pipelines)?,
    );
    let managed = pipeline_managed_guardrails(&pipelines);
    bucket.insert(
        PIPELINE_MANAGED_GUARDRAILS.to_owned(),
        JsonValue::Array(managed.into_iter().map(JsonValue::String).collect()),
    );
    Ok(())
}

fn add_headers(
    data: &mut RetrievalData,
    added: &PolicyPipelines,
    sources: &BTreeMap<String, String>,
) -> Result<(), ResponseRetrievalError> {
    for (policy_name, _) in added {
        add_to_header(data, APPLIED_POLICIES, policy_name)?;
    }
    for (_, pipeline) in added {
        for step in &pipeline.steps {
            add_to_header(data, APPLIED_GUARDRAILS, &step.guardrail)?;
        }
    }
    let mut matched = BTreeMap::new();
    for (policy_name, _) in added {
        if let Some(reason) = sources.get(policy_name) {
            matched.insert(policy_name.clone(), reason.clone());
        }
    }
    add_policy_sources(data, &matched)
}

fn add_to_header(
    data: &mut RetrievalData,
    field: &'static str,
    value: &str,
) -> Result<(), ResponseRetrievalError> {
    let name = metadata_name(data);
    let bucket = bucket_mut(data, name)?;
    let mut entries = existing_string_list(bucket, field)?;
    if entries.iter().any(|entry| entry.as_str() == value) {
        return Ok(());
    }
    entries.push(value.to_owned());
    bucket.insert(
        field.to_owned(),
        JsonValue::Array(entries.into_iter().map(JsonValue::String).collect()),
    );
    Ok(())
}

fn existing_string_list(
    bucket: &BTreeMap<String, JsonValue>,
    field: &'static str,
) -> Result<Vec<String>, ResponseRetrievalError> {
    match bucket.get(field) {
        None | Some(JsonValue::Null) => Ok(Vec::new()),
        Some(JsonValue::Array(items)) => texts(items, field),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) | JsonValue::Object(_),
        ) => Err(ResponseRetrievalError::NotTyped {
            field,
            expected: "a list of strings",
        }),
    }
}

fn texts(items: &[JsonValue], field: &'static str) -> Result<Vec<String>, ResponseRetrievalError> {
    let mut entries = Vec::with_capacity(items.len());
    for item in items {
        let JsonValue::String(entry) = item else {
            return Err(ResponseRetrievalError::NotTyped {
                field,
                expected: "a list of strings",
            });
        };
        entries.push(entry.clone());
    }
    Ok(entries)
}

fn existing_policy_sources(bucket: &BTreeMap<String, JsonValue>) -> BTreeMap<String, JsonValue> {
    if let Some(JsonValue::Object(sources)) = bucket.get(POLICY_SOURCES) {
        return sources.clone();
    }
    BTreeMap::new()
}

fn add_policy_sources(
    data: &mut RetrievalData,
    sources: &BTreeMap<String, String>,
) -> Result<(), ResponseRetrievalError> {
    if sources.is_empty() {
        return Ok(());
    }
    let name = metadata_name(data);
    let bucket = bucket_mut(data, name)?;
    let mut existing = existing_policy_sources(bucket);
    for (policy_name, reason) in sources {
        existing.insert(policy_name.clone(), JsonValue::String(reason.clone()));
    }
    bucket.insert(POLICY_SOURCES.to_owned(), JsonValue::Object(existing));
    Ok(())
}

fn json_from_serialize<T>(value: &T) -> Result<JsonValue, ResponseRetrievalError>
where
    T: Serialize + ?Sized,
{
    match serde_json::to_string(value) {
        Ok(text) => serde_json::from_str(&text).map_err(ResponseRetrievalError::Serialization),
        Err(error) => Err(ResponseRetrievalError::Serialization(error)),
    }
}
