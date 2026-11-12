use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::request_destination::RequestData;

const VECTOR_STORE_ID: &str = "vector_store_id";
const VECTOR_STORE_IDS: &str = "vector_store_ids";
const VECTOR_STORES_SEGMENT: &str = "/vector_stores/";
const LITELLM_METADATA: &str = "litellm_metadata";
const METADATA: &str = "metadata";
const TAGS: &str = "tags";

// body values win over query and path values, as with LiteLLM's setdefault, and the caller's dict is left untouched
pub fn populate_request_parameters(
    request_data: &RequestData,
    path_params: &RequestData,
    query_params: &IndexMap<String, String>,
    route_path: &str,
) -> RequestData {
    let mut populated = request_data.clone();
    for (key, value) in query_params {
        populated
            .entry(key.clone())
            .or_insert_with(|| Json::Text(value.clone()));
    }
    if path_params.is_empty() {
        add_vector_store_id_from_path(&mut populated, route_path);
        return populated;
    }
    for (key, value) in path_params {
        populated
            .entry(key.clone())
            .or_insert_with(|| value.clone());
        if key == VECTOR_STORE_ID {
            merge_vector_store_id(&mut populated, value);
        }
    }
    tracing::debug!(
        "populate_request_with_path_params: Found path_params, vector_store_ids={:?}",
        populated.get(VECTOR_STORE_IDS)
    );
    populated
}

fn add_vector_store_id_from_path(populated: &mut RequestData, route_path: &str) {
    let Some(vector_store_id) = vector_store_id_from_path(route_path) else {
        tracing::debug!(
            "populate_request_with_path_params: No vector_store_id present in path={route_path}"
        );
        return;
    };
    tracing::debug!(
        "populate_request_with_path_params: Extracted vector_store_id={vector_store_id} from path={route_path}"
    );
    let value = Json::Text(vector_store_id.to_owned());
    populated
        .entry(VECTOR_STORE_ID.to_owned())
        .or_insert_with(|| value.clone());
    merge_vector_store_id(populated, &value);
    tracing::debug!(
        "populate_request_with_path_params: Updated request_data with vector_store_ids={:?}",
        populated.get(VECTOR_STORE_IDS)
    );
}

fn merge_vector_store_id(populated: &mut RequestData, vector_store_id: &Json) {
    match populated.get_mut(VECTOR_STORE_IDS) {
        Some(Json::List(ids)) => {
            if !ids.contains(vector_store_id) {
                ids.push(vector_store_id.clone());
            }
        }
        _ => {
            populated.insert(
                VECTOR_STORE_IDS.to_owned(),
                Json::List(vec![vector_store_id.clone()]),
            );
        }
    }
}

// LiteLLM's r"/vector_stores/([^/]+)/": the first such segment that has a nonempty id and a trailing slash
fn vector_store_id_from_path(route_path: &str) -> Option<&str> {
    let mut remainder = route_path;
    while let Some(start) = remainder.find(VECTOR_STORES_SEGMENT) {
        let after_segment = remainder.get(start.checked_add(VECTOR_STORES_SEGMENT.len())?..)?;
        if let Some(end) = after_segment.find('/')
            && end > 0
        {
            return after_segment.get(..end);
        }
        remainder = after_segment;
    }
    None
}

#[must_use]
pub fn tags_from_request_body(request_body: &RequestData) -> Vec<String> {
    let metadata_name = if request_body.contains_key(LITELLM_METADATA) {
        LITELLM_METADATA
    } else {
        METADATA
    };
    let mut tags = Vec::new();
    if let Some(Json::Object(metadata)) = parsed_metadata(request_body.get(metadata_name)) {
        extend_text_tags(metadata.get(TAGS), &mut tags);
    }
    extend_text_tags(request_body.get(TAGS), &mut tags);
    tags
}

// a metadata field may arrive as a JSON string from multipart or extra_body, and anything but an object reads as empty
fn parsed_metadata(metadata: Option<&Json>) -> Option<Json> {
    match metadata {
        Some(Json::Text(text)) => serde_json::from_str::<Json>(text).ok(),
        other => other.cloned(),
    }
}

fn extend_text_tags(values: Option<&Json>, tags: &mut Vec<String>) {
    if let Some(Json::List(items)) = values {
        for item in items {
            if let Json::Text(text) = item {
                tags.push(text.clone());
            }
        }
    }
}
