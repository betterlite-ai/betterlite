use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::message_content::{Content, ContentPart, Message};

use crate::gemini_media::part::{MediaResolution, MediaResolutionLevel};

#[must_use]
pub fn convert_detail(detail: &str) -> Option<MediaResolution> {
    match detail {
        "low" => Some(MediaResolution {
            level: MediaResolutionLevel::Low,
        }),
        "medium" => Some(MediaResolution {
            level: MediaResolutionLevel::Medium,
        }),
        "high" => Some(MediaResolution {
            level: MediaResolutionLevel::High,
        }),
        "ultra_high" => Some(MediaResolution {
            level: MediaResolutionLevel::UltraHigh,
        }),
        _ => None,
    }
}

fn priority(detail: &str) -> u8 {
    match detail {
        "ultra_high" => 4,
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    }
}

#[must_use]
pub fn highest_media_resolution<'detail>(
    current: Option<&'detail str>,
    candidate: Option<&'detail str>,
) -> Option<&'detail str> {
    let current_priority = current.map_or(0, priority);
    let candidate_priority = candidate.map_or(0, priority);
    if candidate_priority > current_priority {
        candidate
    } else {
        current
    }
}

#[must_use]
pub fn max_media_resolution(messages: &[Message]) -> Option<&str> {
    let mut maximum: Option<&str> = None;
    for message in messages {
        let Some(Content::Parts(parts)) = &message.content else {
            continue;
        };
        for part in parts {
            if let Some(detail) = part_detail(part) {
                maximum = highest_media_resolution(maximum, Some(detail));
            }
        }
    }
    maximum
}

fn part_detail(part: &ContentPart) -> Option<&str> {
    let field = match part.kind.as_str() {
        "image_url" => part.fields.get("image_url"),
        "file" => part.fields.get("file"),
        _ => return None,
    };
    field
        .and_then(Json::as_object)
        .and_then(|fields| fields.get("detail"))
        .and_then(Json::as_str)
        .filter(|detail| !detail.is_empty())
}
