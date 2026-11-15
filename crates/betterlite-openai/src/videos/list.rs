use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::openai_video_identifiers::ids::{
    decode_video_id_with_provider, encode_video_id_with_provider,
};
use crate::videos::VideoOptions;
use crate::videos::error::VideoError;

// a list item keeps every key the provider sent; only the id is read to be wrapped
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct VideoListItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(flatten)]
    pub fields: VideoOptions,
}

impl VideoListItem {
    #[must_use]
    pub fn model(&self) -> Option<&str> {
        self.fields.get("model").and_then(Value::as_str)
    }
}

// the list keeps every top-level key the provider sent; data and the cursors are read to be wrapped
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct VideoList {
    #[serde(default)]
    pub data: Vec<VideoListItem>,
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
    if let Some(after) = after {
        let original = decode_video_id_with_provider(after).video_id;
        params.insert("after".to_owned(), Value::String(original));
    }
    if let Some(limit) = limit {
        params.insert("limit".to_owned(), Value::String(limit.to_string()));
    }
    if let Some(order) = order {
        params.insert("order".to_owned(), Value::String(order.to_owned()));
    }
    for (name, value) in extra_query {
        params.insert(name.clone(), value.clone());
    }
    (api_base.to_owned(), params)
}

// transform_video_list_response: ids and cursors are wrapped only inside the data-key branch;
// the encoder leaves an empty provider or id alone, as LiteLLM's truthiness checks do
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
        .and_then(VideoListItem::model)
        .map(str::to_owned);
    let last_model = list
        .data
        .last()
        .and_then(VideoListItem::model)
        .map(str::to_owned);
    for item in &mut list.data {
        let model = item.model().map(str::to_owned);
        if let Some(id) = item.id.as_mut() {
            *id = encode_video_id_with_provider(id, provider_name, model.as_deref());
        }
    }
    for (key, model) in [("first_id", first_model), ("last_id", last_model)] {
        if let Some(Value::String(cursor)) = list.fields.get_mut(key) {
            *cursor = encode_video_id_with_provider(cursor, provider_name, model.as_deref());
        }
    }
    Ok(list)
}
