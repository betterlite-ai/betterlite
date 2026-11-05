use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::FileSanitizer;
use super::data_url::{DataUrl, decode_base64};
use super::error::FileSanitizationError;
use super::wire::Action;

type Item = BTreeMap<String, GuardrailValue>;

struct DocumentFields {
    url: String,
    data: String,
    mime_type: Option<String>,
}

enum Replacement {
    Url(String),
    Data(String),
}

fn string_field(
    fields: &Item,
    field: &'static str,
) -> Result<Option<String>, FileSanitizationError> {
    match fields.get(field) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(FileSanitizationError::InvalidField { field }),
    }
}

fn document_fields(fields: &Item) -> Result<DocumentFields, FileSanitizationError> {
    Ok(DocumentFields {
        url: string_field(fields, "url")?.unwrap_or_default(),
        data: string_field(fields, "data")?.unwrap_or_default(),
        mime_type: string_field(fields, "mime_type")?,
    })
}

fn document_filename(mime_type: &str) -> String {
    if mime_type.contains("pdf") {
        "document.pdf".to_owned()
    } else if mime_type.contains("word") || mime_type.contains("docx") {
        "document.docx".to_owned()
    } else if mime_type.contains("excel") || mime_type.contains("xlsx") {
        "document.xlsx".to_owned()
    } else {
        format!(
            "document.{}",
            mime_type.rsplit('/').next().unwrap_or_default()
        )
    }
}

pub(super) async fn process_document_item(
    sanitizer: &FileSanitizer,
    mut item: Item,
    user_api_key_alias: Option<&str>,
) -> Result<Item, FileSanitizationError> {
    // LiteLLM reads `document`, then `file`, then the item itself.
    let nested = ["document", "file"]
        .into_iter()
        .find(|key| !matches!(item.get(*key), None | Some(GuardrailValue::Null)));
    let fields = read_fields(&item, nested)?;
    if !fields.url.starts_with("data:") && fields.data.is_empty() {
        return Ok(item);
    }
    let outcome = match sanitize_document(sanitizer, &fields, user_api_key_alias).await {
        Ok(outcome) => outcome,
        Err(error) if error.is_http_exception() => return Err(error),
        Err(error) => {
            tracing::error!(%error, "Error sanitizing document");
            return Err(FileSanitizationError::DocumentFailed {
                source: Box::new(error),
            });
        }
    };
    apply_replacement(&mut item, nested, outcome);
    Ok(item)
}

fn read_fields(item: &Item, nested: Option<&str>) -> Result<DocumentFields, FileSanitizationError> {
    let Some(key) = nested else {
        return document_fields(item);
    };
    if let Some(GuardrailValue::Object(inner)) = item.get(key) {
        return document_fields(inner);
    }
    let url = if let Some(GuardrailValue::Text(url)) = item.get(key) {
        url.clone()
    } else {
        String::new()
    };
    Ok(DocumentFields {
        url,
        data: String::new(),
        mime_type: None,
    })
}

fn apply_replacement(item: &mut Item, nested: Option<&str>, outcome: Option<Replacement>) {
    let Some(replacement) = outcome else {
        return;
    };
    let target = match nested {
        None => Some(item),
        Some(key) => match item.get_mut(key) {
            Some(GuardrailValue::Object(inner)) => Some(inner),
            _ => None,
        },
    };
    let Some(slot) = target else {
        return;
    };
    let (field, value) = match replacement {
        Replacement::Url(url) => ("url", url),
        Replacement::Data(data) => ("data", data),
    };
    slot.insert(field.to_owned(), GuardrailValue::Text(value));
}

async fn sanitize_document(
    sanitizer: &FileSanitizer,
    fields: &DocumentFields,
    user_api_key_alias: Option<&str>,
) -> Result<Option<Replacement>, FileSanitizationError> {
    let (header, bytes, mime_type) = if fields.url.starts_with("data:") {
        let data_url = DataUrl::parse(&fields.url)?;
        (data_url.header, data_url.bytes, data_url.mime_type)
    } else {
        let mime_type = fields.mime_type.as_deref().unwrap_or("application/pdf");
        ("", decode_base64(&fields.data)?, mime_type)
    };
    let filename = document_filename(mime_type);
    tracing::info!(filename, "Sanitizing document");
    let result = sanitizer
        .sanitize_file_content(&bytes, &filename, user_api_key_alias)
        .await?;
    sanitizer.raise_if_file_blocked(&result, "Document")?;
    let Some(content) = result.content.filter(|content| !content.is_empty()) else {
        return Ok(None);
    };
    if result.action != Action::Modify {
        return Ok(None);
    }
    tracing::info!("Document content modified by Prompt Security");
    let encoded = STANDARD.encode(content.as_bytes());
    if header.is_empty() {
        Ok(Some(Replacement::Data(encoded)))
    } else {
        Ok(Some(Replacement::Url(format!("{header},{encoded}"))))
    }
}
