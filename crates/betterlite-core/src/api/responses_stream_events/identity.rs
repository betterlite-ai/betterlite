use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;
use betterlite_openai::api::responses_objects::{ResponseOutputItem, ResponsesApiResponse};
use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

use super::record::{Slot, Target, json_of, record_of, record_of_fields, slot_target};
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
    match value {
        Json::Object(fields) => {
            let mut record = record_of_fields(fields.iter());
            rewrite_response_id(&mut record, provider, metadata)?;
            *value = json_of(&record);
            Ok(())
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => Ok(()),
    }
}

pub fn encode(
    event: &mut ResponsesAPIStreamingResponse,
    slot: Slot,
    provider: Option<&str>,
    model_id: Option<&str>,
) -> Result<(), RewriteError> {
    match slot_target(event, slot) {
        Some(target) => encode_target(target, provider, model_id),
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
            let mut value: Json = convert(extra)?;
            encode_value(&mut value, provider, model_id)?;
            *extra = convert(&value)?;
            Ok(())
        }
        Target::Extras(extras) => {
            let mut value: Json = convert(extras)?;
            encode_values(&mut value, provider, model_id)?;
            *extras = convert(&value)?;
            Ok(())
        }
        Target::Value(value) => encode_value(value, provider, model_id),
        Target::Values(values) => encode_values(values, provider, model_id),
    }
}

fn encode_value(
    value: &mut Json,
    provider: Option<&str>,
    model_id: Option<&str>,
) -> Result<(), RewriteError> {
    let mut record = record_of(value);
    encode_container_id(&mut record, provider, model_id);
    *value = json_of(&record);
    Ok(())
}

fn encode_values(
    values: &mut Json,
    provider: Option<&str>,
    model_id: Option<&str>,
) -> Result<(), RewriteError> {
    let Json::List(items) = values else {
        return Ok(());
    };
    for item in items {
        encode_value(item, provider, model_id)?;
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
