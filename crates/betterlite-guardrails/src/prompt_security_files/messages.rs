use std::mem::take;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::FileSanitizer;
use super::data_url::DataUrl;
use super::document::process_document_item;
use super::error::FileSanitizationError;
use super::image::process_image_url_item;

pub async fn process_message_files(
    sanitizer: &FileSanitizer,
    messages: Vec<GuardrailValue>,
    user_api_key_alias: Option<&str>,
) -> Result<Vec<GuardrailValue>, FileSanitizationError> {
    let mut processed = Vec::with_capacity(messages.len());
    for message in messages {
        processed.push(process_message(sanitizer, message, user_api_key_alias).await?);
    }
    Ok(processed)
}

async fn process_message(
    sanitizer: &FileSanitizer,
    mut message: GuardrailValue,
    user_api_key_alias: Option<&str>,
) -> Result<GuardrailValue, FileSanitizationError> {
    if let GuardrailValue::Object(fields) = &mut message
        && let Some(GuardrailValue::Array(items)) = fields.get_mut("content")
    {
        let pending = take(items);
        *items = process_content_items(sanitizer, pending, user_api_key_alias).await?;
    }
    Ok(message)
}

async fn process_content_items(
    sanitizer: &FileSanitizer,
    items: Vec<GuardrailValue>,
    user_api_key_alias: Option<&str>,
) -> Result<Vec<GuardrailValue>, FileSanitizationError> {
    let mut content = Vec::with_capacity(items.len());
    for item in items {
        let GuardrailValue::Object(entry) = item else {
            content.push(item);
            continue;
        };
        let item_type = match entry.get("type") {
            Some(GuardrailValue::Text(item_type)) => item_type.as_str(),
            _ => "",
        };
        let processed = match item_type {
            "image_url" => process_image_url_item(sanitizer, entry, user_api_key_alias).await?,
            "document" | "file" => {
                process_document_item(sanitizer, entry, user_api_key_alias).await?
            }
            _ => entry,
        };
        content.push(GuardrailValue::Object(processed));
    }
    Ok(content)
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
    let result = sanitizer
        .sanitize_file_content(
            &data_url.bytes,
            &data_url.image_filename(),
            user_api_key_alias,
        )
        .await?;
    sanitizer.raise_if_file_blocked(&result, "Image")
}
