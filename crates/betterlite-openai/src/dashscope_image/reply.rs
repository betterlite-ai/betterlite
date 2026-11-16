use betterlite_models::decisions::Json;

use crate::api::image_contracts::records::ImageData;
use crate::dashscope_embedding::error::get_error_class;
use crate::openai_base_config::Headers;

use super::error::DashscopeImageError;

// a reply shape the image records cannot hold is an unclassified provider failure, which LiteLLM answers with 500
const SHAPE_FAILURE_STATUS: u16 = 500;

pub fn decode_dashscope_image_reply(
    bytes: &[u8],
    status_code: u16,
    headers: &Headers,
) -> Result<Vec<ImageData>, DashscopeImageError> {
    if status_code != 200 {
        return Err(DashscopeImageError::Reply(get_error_class(
            &body_text(bytes),
            status_code,
            Some(headers),
        )));
    }
    let body: Json = serde_json::from_slice(bytes).map_err(|source| {
        DashscopeImageError::Reply(get_error_class(
            &format!("Failed to parse DashScope image generation response: {source}"),
            status_code,
            Some(headers),
        ))
    })?;
    let Json::Object(fields) = &body else {
        return Err(shape_error("the reply", headers));
    };
    // DashScope can report an API error inside a 200 body, and only without an output to read
    if fields.contains_key("code") && !fields.contains_key("output") {
        let message = match fields.get("message") {
            Some(Json::Text(text)) => text.clone(),
            _ => serde_json::to_string(&body).unwrap_or_default(),
        };
        return Err(DashscopeImageError::Reply(get_error_class(
            &message,
            status_code,
            Some(headers),
        )));
    }
    let Some(output) = fields.get("output") else {
        return Ok(Vec::new());
    };
    let Json::Object(output) = output else {
        return Err(shape_error("output", headers));
    };
    let Some(choices) = output.get("choices") else {
        return Ok(Vec::new());
    };
    let Json::List(choices) = choices else {
        return Err(shape_error("output.choices", headers));
    };
    let mut data = Vec::new();
    for choice in choices {
        collect_choice(choice, headers, &mut data)?;
    }
    Ok(data)
}

fn collect_choice(
    choice: &Json,
    headers: &Headers,
    data: &mut Vec<ImageData>,
) -> Result<(), DashscopeImageError> {
    let Json::Object(choice) = choice else {
        return Err(shape_error("output.choices", headers));
    };
    let Some(message) = choice.get("message") else {
        return Ok(());
    };
    let Json::Object(message) = message else {
        return Err(shape_error("output.choices.message", headers));
    };
    let Some(content) = message.get("content") else {
        return Ok(());
    };
    let Json::List(content) = content else {
        return Err(shape_error("output.choices.message.content", headers));
    };
    for item in content {
        collect_image(item, headers, data)?;
    }
    Ok(())
}

fn collect_image(
    item: &Json,
    headers: &Headers,
    data: &mut Vec<ImageData>,
) -> Result<(), DashscopeImageError> {
    let Json::Object(item) = item else {
        return Err(shape_error("output.choices.message.content", headers));
    };
    let Some(image) = item.get("image") else {
        return Ok(());
    };
    if image == &Json::Null {
        return Ok(());
    }
    let Json::Text(url) = image else {
        return Err(shape_error("output.choices.message.content.image", headers));
    };
    if url.is_empty() {
        return Ok(());
    }
    data.push(ImageData {
        b64_json: None,
        revised_prompt: None,
        url: Some(url.clone()),
    });
    Ok(())
}

fn body_text(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => format!("{} bytes of a body that is not UTF-8", bytes.len()),
    }
}

fn shape_error(field: &str, headers: &Headers) -> DashscopeImageError {
    DashscopeImageError::Reply(get_error_class(
        &format!("the DashScope image generation reply field `{field}` has the wrong type"),
        SHAPE_FAILURE_STATUS,
        Some(headers),
    ))
}
