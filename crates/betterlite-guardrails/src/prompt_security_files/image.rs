use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::FileSanitizer;
use super::data_url::DataUrl;
use super::error::FileSanitizationError;
use super::wire::Action;

type Item = BTreeMap<String, GuardrailValue>;

pub(super) async fn process_image_url_item(
    sanitizer: &FileSanitizer,
    mut item: Item,
    user_api_key_alias: Option<&str>,
) -> Result<Item, FileSanitizationError> {
    let url = match item.get("image_url") {
        None | Some(GuardrailValue::Null) => String::new(),
        Some(GuardrailValue::Text(url)) => url.clone(),
        Some(GuardrailValue::Object(fields)) => match fields.get("url") {
            None | Some(GuardrailValue::Null) => String::new(),
            Some(GuardrailValue::Text(url)) => url.clone(),
            Some(_) => return Err(FileSanitizationError::InvalidField { field: "url" }),
        },
        Some(_) => return Err(FileSanitizationError::InvalidField { field: "image_url" }),
    };
    if !url.starts_with("data:") {
        return Ok(item);
    }
    match sanitize_image_url(sanitizer, &url, user_api_key_alias).await {
        Ok(Some(sanitized_url)) => {
            match item.get_mut("image_url") {
                Some(GuardrailValue::Object(fields)) => {
                    fields.insert("url".to_owned(), GuardrailValue::Text(sanitized_url));
                }
                Some(slot) => *slot = GuardrailValue::Text(sanitized_url),
                None => {}
            }
            Ok(item)
        }
        Ok(None) => Ok(item),
        Err(error) if error.is_http_exception() => Err(error),
        Err(error) => {
            tracing::error!(%error, "Error sanitizing image file");
            Err(FileSanitizationError::ImageFailed {
                source: Box::new(error),
            })
        }
    }
}

async fn sanitize_image_url(
    sanitizer: &FileSanitizer,
    url: &str,
    user_api_key_alias: Option<&str>,
) -> Result<Option<String>, FileSanitizationError> {
    let data_url = DataUrl::parse(url)?;
    let result = sanitizer
        .sanitize_file_content(
            &data_url.bytes,
            &data_url.image_filename(),
            user_api_key_alias,
        )
        .await?;
    sanitizer.raise_if_file_blocked(&result, "File")?;
    let Some(content) = result.content.filter(|content| !content.is_empty()) else {
        return Ok(None);
    };
    if result.action != Action::Modify {
        return Ok(None);
    }
    tracing::info!("File content modified by Prompt Security");
    Ok(Some(format!(
        "{},{}",
        data_url.header,
        STANDARD.encode(content.as_bytes())
    )))
}
