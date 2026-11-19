use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::{ResponseOutputItem, ResponsesApiResponse};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;

use crate::api::response_id_codecs::response::{build_response_id, is_encoded_response_id};

use super::container::encode_containers;
use super::encrypted::encode_encrypted_item_ids;
use super::metadata::AffinityMetadata;
use super::record::{Record, TraversalError};

#[derive(Debug, Error)]
pub enum ResponseRewriteError {
    #[error("the response could not be encoded: {source}")]
    Encode(#[from] serde_json::Error),
    #[error("the response fields are invalid: {source}")]
    Decode(#[from] serde_path_to_error::Error<serde_json::Error>),
    #[error(transparent)]
    Traversal(#[from] TraversalError),
}

pub fn rewrite_response_id(
    response: &mut Record,
    custom_llm_provider: Option<&str>,
    metadata: &AffinityMetadata,
) -> Result<(), TraversalError> {
    let Some(response_id) = response
        .field("id")
        .and_then(Record::as_text)
        .map(str::to_owned)
    else {
        return Ok(());
    };
    if is_encoded_response_id(&response_id) {
        return Ok(());
    }
    let model_id = metadata
        .model_info
        .as_ref()
        .and_then(|info| info.id.as_deref());
    let updated = build_response_id(custom_llm_provider, model_id, &response_id);
    response.set_text("id", updated)?;
    if metadata.affinity_enabled {
        encode_encrypted_item_ids(response, model_id);
    }
    encode_containers(response, custom_llm_provider, metadata);
    Ok(())
}

// The completed event takes the managed-id rewrite a non-streaming response gets: id, encrypted items, containers
pub fn rewrite_typed_response(
    response: &mut ResponsesApiResponse,
    custom_llm_provider: Option<&str>,
    metadata: &AffinityMetadata,
) -> Result<(), ResponseRewriteError> {
    let mut value: Json = transcode(response)?;
    rewrite_value(&mut value, custom_llm_provider, metadata)?;
    let rewritten: RewrittenFields = transcode(&value)?;
    response.id = rewritten.id;
    response.output = Vec::with_capacity(rewritten.output.len());
    for item in rewritten.output {
        response.output.push(ResponseOutputItem::Unknown(item));
    }
    Ok(())
}

// B904's rewrite changes only the id and the output items, which stay raw so a typed re-parse adds no fields
#[derive(Deserialize)]
struct RewrittenFields {
    id: String,
    output: Vec<BTreeMap<String, Json>>,
}

// The response value of a generic event gets the same rewrite; the rewrite ignores a value that is not an object
pub fn rewrite_value(
    value: &mut Json,
    custom_llm_provider: Option<&str>,
    metadata: &AffinityMetadata,
) -> Result<(), ResponseRewriteError> {
    let mut record = Record::from_json(value.clone());
    rewrite_response_id(&mut record, custom_llm_provider, metadata)?;
    *value = record.into_json();
    Ok(())
}

// Every conversion here crosses JSON, so one serde pass each way and a failure names its field
pub fn transcode<Source, Target>(source: &Source) -> Result<Target, ResponseRewriteError>
where
    Source: Serialize + ?Sized,
    Target: DeserializeOwned,
{
    let encoded = serde_json::to_vec(source)?;
    serde_path_to_error::deserialize(&mut serde_json::Deserializer::from_slice(&encoded))
        .map_err(ResponseRewriteError::from)
}
