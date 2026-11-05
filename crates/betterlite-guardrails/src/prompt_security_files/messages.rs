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
        process_content_items(sanitizer, items, user_api_key_alias).await?;
    }
    Ok(message)
}

async fn process_content_items(
    sanitizer: &FileSanitizer,
    items: &mut [GuardrailValue],
    user_api_key_alias: Option<&str>,
) -> Result<(), FileSanitizationError> {
    for slot in items {
        let GuardrailValue::Object(entry) = slot else {
            continue;
        };
        let fields = take(entry);
        let item_type = match fields.get("type") {
            Some(GuardrailValue::Text(item_type)) => item_type.clone(),
            _ => String::new(),
        };
        *entry = match item_type.as_str() {
            "image_url" => process_image_url_item(sanitizer, fields, user_api_key_alias).await?,
            "document" | "file" => {
                process_document_item(sanitizer, fields, user_api_key_alias).await?
            }
            _ => fields,
        };
    }
    Ok(())
}

pub async fn process_standalone_images(
    sanitizer: &FileSanitizer,
    images: &[String],
    user_api_key_alias: Option<&str>,
) -> Result<(), FileSanitizationError> {
    for image_url in images.iter().filter(|url| url.starts_with("data:")) {
        let outcome = match DataUrl::parse(image_url) {
            Ok(data_url) => {
                let filename = data_url.image_filename();
                sanitizer
                    .sanitize_file_content(&data_url.bytes, &filename, user_api_key_alias)
                    .await
                    .and_then(|result| sanitizer.raise_if_file_blocked(&result, "Image"))
            }
            Err(error) => Err(error),
        };
        if let Err(error) = outcome {
            if error.is_http_exception() {
                return Err(error);
            }
            tracing::error!(%error, "Error processing image");
        }
    }
    Ok(())
}
