use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::basic::VectorStoreStatus;
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreCreateResponse, VectorStoreFileCounts,
};
use indexmap::IndexMap;

use super::decode::{RawResponse, reply_text};
use super::error::StudioFileSearchError;
use crate::provider_metadata::timestamp::LocalTimeContext;
use crate::vertex_fine_tuning::records::timestamp::created_at;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatePlan {
    pub url: String,
    pub body: Json,
}

// transform_create_vector_store_request writes displayName only for a name it can see.
#[must_use]
pub fn create_request(name: Option<&str>, api_base: &str) -> CreatePlan {
    let mut body = IndexMap::new();
    if let Some(name) = name.filter(|name| !name.is_empty()) {
        body.insert("displayName".to_owned(), Json::Text(name.to_owned()));
    }
    CreatePlan {
        url: format!("{api_base}/fileSearchStores"),
        body: Json::Object(body),
    }
}

pub fn decode_create(
    response: &RawResponse<'_>,
    local: &LocalTimeContext,
) -> Result<VectorStoreCreateResponse, StudioFileSearchError> {
    let parsed = serde_json::from_slice::<Json>(response.body)
        .map_err(|error| response.create_failure(error.to_string()))?;
    let Json::Object(fields) = parsed else {
        return Err(response.create_failure("the reply is not an object".to_owned()));
    };
    let store_name =
        reply_text(&fields, "name").map_err(|error| response.create_failure(error.to_string()))?;
    let display_name = reply_text(&fields, "displayName")
        .map_err(|error| response.create_failure(error.to_string()))?;
    let create_time = reply_text(&fields, "createTime")
        .map_err(|error| response.create_failure(error.to_string()))?;
    let mut extra = BTreeMap::new();
    for key in ["expires_after", "expires_at", "last_active_at", "metadata"] {
        extra.insert(key.to_owned(), serde_json::to_value(Json::Null)?);
    }
    Ok(VectorStoreCreateResponse {
        id: Some(store_name.to_owned()),
        object: Some("vector_store".to_owned()),
        created_at: Some(created_at(
            Some(create_time).filter(|time| !time.is_empty()),
            local,
        )),
        name: Some(display_name.to_owned()),
        bytes: Some(0),
        file_counts: Some(VectorStoreFileCounts {
            in_progress: Some(0),
            completed: Some(0),
            failed: Some(0),
            cancelled: Some(0),
            total: Some(0),
            extra: BTreeMap::new(),
        }),
        status: Some(VectorStoreStatus::Completed),
        expires_after: None,
        expires_at: None,
        last_active_at: None,
        metadata: None,
        extra,
    })
}
