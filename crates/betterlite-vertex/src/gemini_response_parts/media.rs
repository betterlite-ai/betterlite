use crate::gemini_response_parts::projection::{Part, PartKind, PartsError};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::Serialize;
use uuid::Uuid;
const AUDIO_TTL_SECONDS: i64 = 86_400;
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImagePart {
    pub image_url: ImageUrl,
    pub index: u32,
    #[serde(rename = "type")]
    pub kind: PartKind,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImageUrl {
    pub url: String,
    pub detail: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AudioPart {
    pub id: String,
    pub data: String,
    pub expires_at: i64,
    pub transcript: String,
}
pub(crate) fn image_parts(parts: &[Part]) -> Vec<ImagePart> {
    parts
        .iter()
        .filter_map(|part| {
            let inline = part.inline_data.as_ref()?;
            inline.mime_type.starts_with("image/").then(|| ImagePart {
                image_url: ImageUrl {
                    url: format!("data:{};base64,{}", inline.mime_type, inline.data),
                    detail: "auto".to_owned(),
                },
                index: 0,
                kind: PartKind::ImageUrl,
            })
        })
        .collect()
}
pub(crate) fn audio_part(
    parts: &[Part],
    now: i64,
    ids: &mut dyn FnMut() -> Uuid,
) -> Result<Option<AudioPart>, PartsError> {
    let data = parts.iter().find_map(|part| {
        if let Some(text) = &part.text {
            audio_uri(text).map(|(_, data)| data.to_owned())
        } else {
            part.inline_data
                .as_ref()
                .filter(|inline| inline.mime_type.starts_with("audio/"))
                .map(|inline| inline.data.clone())
        }
    });
    let Some(data) = data else {
        return Ok(None);
    };
    Ok(Some(AudioPart {
        id: ids().to_string(),
        data,
        expires_at: now
            .checked_add(AUDIO_TTL_SECONDS)
            .ok_or(PartsError::Expiry { now })?,
        transcript: String::new(),
    }))
}
pub(super) fn audio_uri(text: &str) -> Option<(&str, &str)> {
    let (_, rest) = text.split_once("data:")?;
    if !text.starts_with("data:audio") || !text.contains(";base64,") {
        return None;
    }
    let payload = rest.split(',').nth(1).unwrap_or_default();
    if !STANDARD
        .decode(payload)
        .is_ok_and(|decoded| STANDARD.encode(decoded) == payload)
    {
        return None;
    }
    let segment = rest.split_once("data:").map_or(rest, |(before, _)| before);
    let (media_type, data) = segment.split_once(";base64,")?;
    media_type
        .starts_with("audio/")
        .then_some((media_type, data))
}
