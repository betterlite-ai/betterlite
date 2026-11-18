use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;
use betterlite_openai::api::responses_objects::{ResponseOutputItem, ResponsesApiResponse};
use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

use super::record::{
    PartTarget, Target, annotation_target, extra_of, item_target, json_of, part_target, record_of,
    record_of_fields,
};
use crate::api::response_id_traversal::container::encode_container_id;
use crate::api::response_id_traversal::metadata::AffinityMetadata;
use crate::api::response_id_traversal::record::TraversalError;
use crate::api::response_id_traversal::response_id::rewrite_response_id;

#[derive(Debug, Error)]
pub enum RewriteError {
    #[error("the streaming event could not be re-encoded")]
    Encoding(#[from] serde_json::Error),
    #[error("the streaming event id could not be rewritten")]
    Traversal(#[from] TraversalError),
}

// the rewrite touches only these two fields, so the typed response keeps its own extra fields
#[derive(serde::Deserialize)]
struct RewrittenFields {
    id: String,
    output: Vec<BTreeMap<String, Json>>,
}

pub fn rewrite_response(
    response: &mut ResponsesApiResponse,
    provider: Option<&str>,
    metadata: &AffinityMetadata,
) -> Result<(), RewriteError> {
    let response_json: Json = convert(response)?;
    let mut record = record_of(&response_json);
    rewrite_response_id(&mut record, provider, metadata)?;
    let rewritten: RewrittenFields = convert(&json_of(&record))?;
    response.id = rewritten.id;
    response.output = rewritten
        .output
        .into_iter()
        .map(ResponseOutputItem::Unknown)
        .collect();
    Ok(())
}

pub fn rewrite_response_value(
    value: &mut Json,
    provider: Option<&str>,
    metadata: &AffinityMetadata,
) -> Result<(), RewriteError> {
    if !matches!(value, Json::Object(_)) {
        return Ok(());
    }
    let mut record = record_of(value);
    rewrite_response_id(&mut record, provider, metadata)?;
    *value = json_of(&record);
    Ok(())
}

pub fn encode_item(
    event: &mut ResponsesAPIStreamingResponse,
    provider: Option<&str>,
    model_id: Option<&str>,
) -> Result<(), RewriteError> {
    match item_target(event) {
        Some(target) => encode_target(target, provider, model_id),
        None => Ok(()),
    }
}

pub fn encode_annotation(
    event: &mut ResponsesAPIStreamingResponse,
    provider: Option<&str>,
    model_id: Option<&str>,
) -> Result<(), RewriteError> {
    match annotation_target(event) {
        Some(target) => encode_target(target, provider, model_id),
        None => Ok(()),
    }
}

pub fn encode_part(
    event: &mut ResponsesAPIStreamingResponse,
    provider: Option<&str>,
    model_id: Option<&str>,
) -> Result<(), RewriteError> {
    match part_target(event) {
        Some(PartTarget::Extra(annotations)) => {
            let mut values: Vec<Json> = convert(annotations)?;
            encode_values(&mut values, provider, model_id)?;
            *annotations = convert(&values)?;
            Ok(())
        }
        Some(PartTarget::Value(value)) => {
            let Json::Object(fields) = value else {
                return Ok(());
            };
            let Some(Json::List(annotations)) = fields.get_mut("annotations") else {
                return Ok(());
            };
            encode_values(annotations, provider, model_id)
        }
        None => Ok(()),
    }
}

fn encode_target(
    target: Target<'_>,
    provider: Option<&str>,
    model_id: Option<&str>,
) -> Result<(), RewriteError> {
    match target {
        Target::Extra(extra) => {
            let mut record = record_of_fields(extra.iter());
            encode_container_id(&mut record, provider, model_id);
            if let Some(encoded) = extra_of(&record) {
                *extra = encoded;
            }
            Ok(())
        }
        Target::Value(value) => {
            let mut record = record_of(value);
            encode_container_id(&mut record, provider, model_id);
            *value = json_of(&record);
            Ok(())
        }
    }
}

fn encode_values(
    values: &mut [Json],
    provider: Option<&str>,
    model_id: Option<&str>,
) -> Result<(), RewriteError> {
    for value in values {
        encode_target(Target::Value(value), provider, model_id)?;
    }
    Ok(())
}

fn convert<Source, Target>(source: &Source) -> Result<Target, RewriteError>
where
    Source: Serialize + ?Sized,
    Target: DeserializeOwned,
{
    let encoded = serde_json::to_vec(source)?;
    let decoded = serde_json::from_slice(&encoded)?;
    Ok(decoded)
}
