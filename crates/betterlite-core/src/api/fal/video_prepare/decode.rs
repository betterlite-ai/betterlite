use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::VideoHeaders;
use super::error::FalVideoError;

pub const MAX_FAL_VIDEO_REPLY_BYTES: usize = 0x0100_0000;

// the fields B1533 turns into status replies and error text, kept typed so neither task re-parses the body
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FalVideoReply {
    pub status: Option<String>,
    pub request_id: Option<String>,
    pub video: Option<FalVideoResult>,
    pub detail: Option<FalVideoDetail>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FalVideoResult {
    pub url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FalVideoDetail {
    Text(String),
    Items(Vec<FalVideoDetailItem>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FalVideoDetailItem {
    pub msg: Option<String>,
    pub loc: Option<FalVideoLocation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FalVideoLocation {
    Text(String),
    Parts(Vec<String>),
}

pub fn decode_fal_video_reply(
    status: u16,
    headers: &VideoHeaders,
    body: &[u8],
) -> Result<FalVideoReply, FalVideoError> {
    if body.len() > MAX_FAL_VIDEO_REPLY_BYTES {
        return Err(FalVideoError::ReplyTooLarge {
            status,
            headers: headers.clone(),
            limit: MAX_FAL_VIDEO_REPLY_BYTES,
        });
    }
    let value: Json =
        serde_json::from_slice(body).map_err(|source| FalVideoError::ReplyDecode {
            status,
            headers: headers.clone(),
            source,
        })?;
    let Json::Object(fields) = value else {
        return Err(FalVideoError::ReplyNotAnObject {
            status,
            headers: headers.clone(),
        });
    };
    Ok(FalVideoReply {
        status: text_field(&fields, "status"),
        request_id: text_field(&fields, "request_id"),
        video: video_result(fields.get("video")),
        detail: detail(fields.get("detail")),
        error: text_field(&fields, "error"),
    })
}

fn text_field(fields: &IndexMap<String, Json>, name: &str) -> Option<String> {
    match fields.get(name) {
        Some(Json::Text(text)) => Some(text.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn video_result(value: Option<&Json>) -> Option<FalVideoResult> {
    let Json::Object(fields) = value? else {
        return None;
    };
    let url = text_field(fields, "url").filter(|url| !url.is_empty());
    Some(FalVideoResult { url })
}

fn detail(value: Option<&Json>) -> Option<FalVideoDetail> {
    let present = value?;
    if let Json::Text(text) = present {
        return Some(FalVideoDetail::Text(text.clone()));
    }
    let Json::List(entries) = present else {
        return None;
    };
    Some(FalVideoDetail::Items(
        entries.iter().filter_map(detail_item).collect(),
    ))
}

fn detail_item(item: &Json) -> Option<FalVideoDetailItem> {
    let Json::Object(fields) = item else {
        return None;
    };
    Some(FalVideoDetailItem {
        msg: text_field(fields, "msg"),
        loc: location(fields.get("loc")),
    })
}

fn location(value: Option<&Json>) -> Option<FalVideoLocation> {
    match value? {
        Json::Text(text) if !text.is_empty() => Some(FalVideoLocation::Text(text.clone())),
        Json::List(parts) => {
            let locations: Vec<String> = parts.iter().filter_map(location_part).collect();
            (!locations.is_empty()).then_some(FalVideoLocation::Parts(locations))
        }
        Json::Text(_) | Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => None,
    }
}

fn location_part(part: &Json) -> Option<String> {
    match part {
        Json::Text(text) => Some(text.clone()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}
