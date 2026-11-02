use betterlite_ocr::json::{Json, Object};

use super::OcrDocument;
use super::error::OcrRequestError;

#[derive(Debug)]
pub(super) struct JsonBody {
    pub(super) document: Option<OcrDocument>,
    pub(super) fields: Object,
}

pub(super) fn parse(body: &[u8]) -> Result<JsonBody, OcrRequestError> {
    let value: Json = serde_json::from_slice(body).map_err(OcrRequestError::InvalidJson)?;
    let Json::Object(mut fields) = value else {
        return Err(OcrRequestError::NotAJsonObject);
    };
    let document = match fields.get("document") {
        Some(Json::Object(doc)) => Some(validate_document(doc)?),
        _ => None,
    };
    if document.is_some() {
        fields.shift_remove("document");
    }
    Ok(JsonBody { document, fields })
}

// "file" documents assume a shared filesystem, and reducto:// ids are not scoped to the proxy user.
fn validate_document(doc: &Object) -> Result<OcrDocument, OcrRequestError> {
    if matches!(doc.get("type"), Some(Json::Text(kind)) if kind == "file") {
        return Err(OcrRequestError::FileTypeNotSupported);
    }
    for field in ["document_url", "image_url"] {
        if let Some(Json::Text(url)) = doc.get(field)
            && url.starts_with("reducto://")
        {
            return Err(OcrRequestError::ReductoIdRejected);
        }
    }
    Ok(OcrDocument::Inline(doc.clone()))
}

mod tests;
