#![cfg(test)]

use super::*;

fn part(kind: &str, fields: Object) -> ContentPart {
    ContentPart {
        kind: kind.to_owned(),
        fields,
    }
}

fn message(parts: Vec<ContentPart>) -> Message {
    Message {
        role: "user".to_owned(),
        content: Some(Content::Parts(parts)),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

mod anthropic_image_source;
mod image_detection_and_projection;
mod image_part_url_dialects;
mod image_scan_depth;
mod image_scan_depth_bound;
