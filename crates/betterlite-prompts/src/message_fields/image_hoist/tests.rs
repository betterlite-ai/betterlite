#![cfg(test)]

use super::*;

fn part(kind: &str, fields: Object) -> ContentPart {
    ContentPart {
        kind: kind.to_owned(),
        fields,
    }
}

fn message(role: &str, parts: Vec<ContentPart>) -> Message {
    Message {
        role: role.to_owned(),
        content: Some(Content::Parts(parts)),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

fn image_url_part_string(url: &str) -> ContentPart {
    part(
        "image_url",
        Object::from_iter([("image_url".to_owned(), Json::Text(url.to_owned()))]),
    )
}

fn image_url_part_object(url: &str) -> ContentPart {
    part(
        "image_url",
        Object::from_iter([(
            "image_url".to_owned(),
            Json::Object(Object::from_iter([(
                "url".to_owned(),
                Json::Text(url.to_owned()),
            )])),
        )]),
    )
}

mod extracts_image_urls_from_content_parts;
mod hoists_images_out_of_tool_message_runs;
