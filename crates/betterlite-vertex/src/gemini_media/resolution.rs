use crate::gemini_media::part::{MediaResolution, MediaResolutionLevel};
use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::message_content::{Content, ContentPart, Message};
const DETAILS: &[(&str, MediaResolutionLevel, u8)] = &[
    ("low", MediaResolutionLevel::Low, 1),
    ("medium", MediaResolutionLevel::Medium, 2),
    ("high", MediaResolutionLevel::High, 3),
    ("ultra_high", MediaResolutionLevel::UltraHigh, 4),
];
#[must_use]
pub fn convert_detail(detail: &str) -> Option<MediaResolution> {
    DETAILS
        .iter()
        .find(|(name, _level, _priority)| *name == detail)
        .map(|(_name, level, _priority)| MediaResolution { level: *level })
}
#[must_use]
pub fn max_media_resolution(messages: &[Message]) -> Option<&str> {
    let mut maximum: Option<&str> = None;
    let mut maximum_priority = 0;
    for message in messages {
        let Some(Content::Parts(parts)) = &message.content else {
            continue;
        };
        for part in parts {
            let Some(detail) = part_detail(part) else {
                continue;
            };
            let priority = detail_priority(detail);
            if priority > maximum_priority {
                maximum = Some(detail);
                maximum_priority = priority;
            }
        }
    }
    maximum
}
fn detail_priority(detail: &str) -> u8 {
    DETAILS
        .iter()
        .find(|(name, _level, _priority)| *name == detail)
        .map_or(0, |(_name, _level, priority)| *priority)
}
fn part_detail(part: &ContentPart) -> Option<&str> {
    let field = match part.kind.as_str() {
        "image_url" | "file" => part.fields.get(part.kind.as_str()),
        _ => return None,
    };
    field
        .and_then(Json::as_object)
        .and_then(|fields| fields.get("detail"))
        .and_then(Json::as_str)
        .filter(|detail| !detail.is_empty())
}
