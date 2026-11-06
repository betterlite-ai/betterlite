use std::collections::BTreeMap;
use std::mem::take;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::FileSanitizer;
use super::data_url::DataUrl;
use super::document::process_document_item;
use super::error::FileSanitizationError;
use super::image::process_image_url_item;

pub async fn process_message_files(
    sanitizer: &FileSanitizer,
    mut messages: Vec<GuardrailValue>,
    user_api_key_alias: Option<&str>,
) -> Result<Vec<GuardrailValue>, FileSanitizationError> {
    for message in &mut messages {
        if let GuardrailValue::Object(fields) = message
            && let Some(GuardrailValue::Array(items)) = fields.get_mut("content")
        {
            process_content_items(sanitizer, items, user_api_key_alias).await?;
        }
    }
    Ok(messages)
}

async fn process_content_items(
    sanitizer: &FileSanitizer,
    items: &mut [GuardrailValue],
    user_api_key_alias: Option<&str>,
) -> Result<(), FileSanitizationError> {
    for slot in items {
        if let GuardrailValue::Object(entry) = slot {
            *entry = sanitize_content_item(sanitizer, take(entry), user_api_key_alias).await?;
        }
    }
    Ok(())
}

async fn sanitize_content_item(
    sanitizer: &FileSanitizer,
    fields: BTreeMap<String, GuardrailValue>,
    user_api_key_alias: Option<&str>,
) -> Result<BTreeMap<String, GuardrailValue>, FileSanitizationError> {
    let item_type = match fields.get("type") {
        Some(GuardrailValue::Text(item_type)) => item_type.clone(),
        _ => String::new(),
    };
    match item_type.as_str() {
        "image_url" => process_image_url_item(sanitizer, fields, user_api_key_alias).await,
        "document" | "file" => process_document_item(sanitizer, fields, user_api_key_alias).await,
        _ => Ok(fields),
    }
}

pub async fn process_standalone_images(
    sanitizer: &FileSanitizer,
    images: &[String],
    user_api_key_alias: Option<&str>,
) -> Result<(), FileSanitizationError> {
    for image_url in images.iter().filter(|url| url.starts_with("data:")) {
        let Err(error) = sanitize_standalone_image(sanitizer, image_url, user_api_key_alias).await
        else {
            continue;
        };
        if error.is_http_exception() {
            return Err(error);
        }
        tracing::error!(%error, "Error processing image");
    }
    Ok(())
}

async fn sanitize_standalone_image(
    sanitizer: &FileSanitizer,
    image_url: &str,
    user_api_key_alias: Option<&str>,
) -> Result<(), FileSanitizationError> {
    let data_url = DataUrl::parse(image_url)?;
    let filename = data_url.image_filename();
    let result = sanitizer
        .sanitize_file_content(&data_url.bytes, &filename, user_api_key_alias)
        .await?;
    sanitizer.raise_if_file_blocked(&result, "Image")
}
