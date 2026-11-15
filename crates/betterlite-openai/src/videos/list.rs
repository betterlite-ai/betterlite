use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::openai_video_identifiers::ids::encode_video_id_with_provider;
use crate::videos::VideoOptions;
use crate::videos::error::VideoError;
use crate::videos::reply::{VideoObject, decode_value};
use crate::videos::segment::original_video_id;

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct VideoList {
    #[serde(default)]
    pub data: Vec<VideoObject>,
    pub first_id: Option<String>,
    pub last_id: Option<String>,
    pub object: Option<String>,
    pub has_more: Option<bool>,
}

pub(crate) fn carries_list_data(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|fields| fields.get("data").is_some_and(Value::is_array))
}

pub(crate) fn decode_list(value: Value) -> Result<VideoList, VideoError> {
    serde_json::from_value(value).map_err(VideoError::from)
}

// transform_video_list_request: api_base stays exactly as given, and after is decoded first
#[must_use]
pub fn list_request(
    api_base: &str,
    after: Option<&str>,
    limit: Option<i64>,
    order: Option<&str>,
    extra_query: Option<&VideoOptions>,
) -> (String, VideoOptions) {
    let mut params = VideoOptions::new();
    if let Some(after) = after {
        params.insert("after".to_owned(), Value::String(original_video_id(after)));
    }
    if let Some(limit) = limit {
        params.insert("limit".to_owned(), Value::String(limit.to_string()));
    }
    if let Some(order) = order {
        params.insert("order".to_owned(), Value::String(order.to_owned()));
    }
    if let Some(extra_query) = extra_query {
        for (name, value) in extra_query {
            params.insert(name.clone(), value.clone());
        }
    }
    (api_base.to_owned(), params)
}

// transform_video_list_response: ids and cursors are wrapped only inside the data-key branch
pub fn list_response(body: &[u8], provider: Option<&str>) -> Result<VideoList, VideoError> {
    let value = decode_value(body)?;
    let in_data_branch = carries_list_data(&value);
    let mut list = decode_list(value)?;
    let Some(provider) = provider.filter(|value| !value.is_empty()) else {
        return Ok(list);
    };
    if !in_data_branch {
        return Ok(list);
    }
    let first_model = list.data.first().and_then(|video| video.model.clone());
    let last_model = list.data.last().and_then(|video| video.model.clone());
    for video in &mut list.data {
        video.id = encode_video_id_with_provider(&video.id, provider, video.model.as_deref());
    }
    let first = list.first_id.clone().filter(|value| !value.is_empty());
    if let Some(first) = first {
        list.first_id = Some(encode_video_id_with_provider(
            &first,
            provider,
            first_model.as_deref(),
        ));
    }
    let last = list.last_id.clone().filter(|value| !value.is_empty());
    if let Some(last) = last {
        list.last_id = Some(encode_video_id_with_provider(
            &last,
            provider,
            last_model.as_deref(),
        ));
    }
    Ok(list)
}
