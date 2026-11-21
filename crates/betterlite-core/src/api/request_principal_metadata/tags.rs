use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::api_boundaries::ApiLimits;
use crate::api::request_destination::RequestData;
use crate::api::request_metadata_safety::metadata::{
    MetadataField, metadata_name_from_data, normalized_metadata_slot,
};
use crate::api::request_principal_metadata::{Metadata, take_bucket};

pub const TAGS_HEADER: &str = "x-litellm-tags";
pub const TAGS_KEY: &str = "tags";
// LITELLM_METADATA_ROUTES alone; the thread and assistant routes only pick the slot, they never seed it
const LITELLM_METADATA_ROUTES: [&str; 5] =
    ["batches", "bedrock", "/v1/messages", "responses", "files"];

// the tags already on the request come first and keep their duplicates; only added tags are deduplicated
#[must_use]
pub fn merge_tags(request_tags: Option<&Json>, tags_to_add: &[Json]) -> Vec<Json> {
    let mut merged = if let Some(Json::List(tags)) = request_tags {
        tags.clone()
    } else {
        Vec::new()
    };
    for tag in tags_to_add {
        if !merged.contains(tag) {
            merged.push(tag.clone());
        }
    }
    merged
}

// a list of tags in the body replaces the header's tags, and the header keeps the empty entries it splits into
#[must_use]
pub fn request_tags(headers: &IndexMap<String, String>, data: &RequestData) -> Option<Vec<Json>> {
    if let Some(Json::List(tags)) = data.get(TAGS_KEY) {
        return Some(tags.clone());
    }
    let header = tags_header(headers)?;
    Some(
        header
            .split(',')
            .map(|tag| Json::Text(tag.trim().to_owned()))
            .collect(),
    )
}

pub fn pre_seed_litellm_metadata(data: &mut RequestData, route: &str) {
    if LITELLM_METADATA_ROUTES
        .iter()
        .any(|metadata_route| route.contains(metadata_route))
    {
        data.entry(MetadataField::LitellmMetadata.as_str().to_owned())
            .or_insert_with(|| Json::Object(IndexMap::new()));
    }
}

pub fn apply_key_tags(data: &mut RequestData, key_metadata: &Metadata, limits: ApiLimits) {
    let Some(Json::List(key_tags)) = key_metadata.get(TAGS_KEY) else {
        return;
    };
    if key_tags.is_empty() {
        return;
    }
    merge_into_request_slot(data, key_tags, limits);
}

// the pre-auth merge drops the empty entries a header like "a,,b" splits into
pub fn apply_header_tags(
    data: &mut RequestData,
    headers: &IndexMap<String, String>,
    limits: ApiLimits,
) {
    let Some(header) = tags_header(headers) else {
        return;
    };
    let header_tags: Vec<Json> = header
        .split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(|tag| Json::Text(tag.to_owned()))
        .collect();
    if header_tags.is_empty() {
        return;
    }
    merge_into_request_slot(data, &header_tags, limits);
}

fn merge_into_request_slot(data: &mut RequestData, tags: &[Json], limits: ApiLimits) {
    let slot = metadata_name_from_data(data);
    normalized_metadata_slot(data, slot, limits);
    let mut bucket = take_bucket(data, slot);
    let merged = merge_tags(bucket.get(TAGS_KEY), tags);
    bucket.insert(TAGS_KEY.to_owned(), Json::List(merged));
    data.insert(slot.as_str().to_owned(), Json::Object(bucket));
}

fn tags_header(headers: &IndexMap<String, String>) -> Option<&str> {
    headers
        .iter()
        .find(|(name, _value)| name.eq_ignore_ascii_case(TAGS_HEADER))
        .map(|(_name, value)| value.as_str())
}
