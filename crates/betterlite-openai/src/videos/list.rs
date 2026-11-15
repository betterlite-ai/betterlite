use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::openai_video_identifiers::ids::{
    decode_video_id_with_provider, encode_video_id_with_provider,
};
use crate::videos::VideoOptions;
use crate::videos::error::VideoError;

// a list item keeps every key the provider sent; id and model are the keys the wrap reads
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct VideoListItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub model: Option<Option<String>>,
    #[serde(flatten)]
    pub fields: VideoOptions,
}

// the list keeps every top-level key the provider sent; the declared ones are the keys the wrap reads
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct VideoList {
    #[serde(default)]
    pub data: Vec<VideoListItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub first_id: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub last_id: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,
    #[serde(flatten)]
    pub fields: VideoOptions,
}

pub(crate) fn carries_list_data(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|fields| fields.get("data").is_some_and(Value::is_array))
}

// transform_video_list_request: api_base stays exactly as given, and after is decoded first
#[must_use]
pub fn list_request(
    api_base: &str,
    after: Option<&str>,
    limit: Option<i64>,
    order: Option<&str>,
    extra_query: &VideoOptions,
) -> (String, VideoOptions) {
    let mut params = VideoOptions::new();
    if let Some(cursor) = after {
        let original = decode_video_id_with_provider(cursor).video_id;
        params.insert("after".to_owned(), Value::String(original));
    }
    if let Some(count) = limit {
        params.insert("limit".to_owned(), Value::String(count.to_string()));
    }
    if let Some(direction) = order {
        params.insert("order".to_owned(), Value::String(direction.to_owned()));
    }
    for (name, value) in extra_query {
        params.insert(name.clone(), value.clone());
    }
    (api_base.to_owned(), params)
}

// transform_video_list_response: ids and cursors are wrapped only inside the data-key branch
pub fn list_response(body: &[u8], provider: Option<&str>) -> Result<VideoList, VideoError> {
    let json = serde_json::from_slice::<Value>(body)?;
    let in_data_branch = carries_list_data(&json);
    let mut list = serde_json::from_value::<VideoList>(json)?;
    let Some(provider_name) = provider else {
        return Ok(list);
    };
    if !in_data_branch {
        return Ok(list);
    }
    let first_model = list
        .data
        .first()
        .and_then(|item| item.model.clone().flatten());
    let last_model = list
        .data
        .last()
        .and_then(|item| item.model.clone().flatten());
    for item in &mut list.data {
        let model = item.model.clone().flatten();
        if let Some(id) = item.id.as_mut() {
            *id = encode_video_id_with_provider(id, provider_name, model.as_deref());
        }
    }
    if let Some(Some(cursor)) = list.first_id.as_mut() {
        *cursor = encode_video_id_with_provider(cursor, provider_name, first_model.as_deref());
    }
    if let Some(Some(cursor)) = list.last_id.as_mut() {
        *cursor = encode_video_id_with_provider(cursor, provider_name, last_model.as_deref());
    }
    Ok(list)
}
