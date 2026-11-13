use crate::api::response_id_codecs::response::{build_response_id, is_encoded_response_id};

use super::container::encode_containers;
use super::encrypted::encode_encrypted_item_ids;
use super::metadata::AffinityMetadata;
use super::record::{Record, TraversalError};

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
