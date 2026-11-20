use std::collections::BTreeMap;

use uuid::Uuid;

use crate::api::request_body::JsonObject;
use crate::api::request_form::values::{FormParts, UploadFile};
use crate::rag::upload_security::{MalwareScanner, UploadValidation, validate_upload};

pub mod base64;
mod credentials;
pub mod error;
mod fields;
mod tests;

use self::error::IngestRequestError;

pub struct IngestRequest<'request> {
    pub headers: &'request BTreeMap<String, String>,
    pub body: IngestBody<'request>,
}

#[derive(Debug, Clone, Copy)]
pub enum IngestBody<'request> {
    Multipart(&'request FormParts),
    Json(&'request JsonObject),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedIngestRequest {
    pub ingest_options: JsonObject,
    pub file: Option<SecuredDocument>,
    pub file_url: Option<String>,
    pub file_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecuredDocument {
    pub safe_filename: String,
    pub content: Vec<u8>,
    pub content_type: String,
}

// endpoints.py:404-525 behind a typed body: the content type picks the branch, uploads are secured before the ingest options are inspected
pub fn parse_rag_ingest_request<Id>(
    request: &IngestRequest<'_>,
    max_upload_bytes: usize,
    scanner: &dyn MalwareScanner,
    new_id: Id,
) -> Result<ParsedIngestRequest, IngestRequestError>
where
    Id: FnOnce() -> Uuid,
{
    let content_type = request
        .headers
        .get("content-type")
        .map_or("", String::as_str);
    // endpoints.py:431 matches a raw substring, so a differently cased media type takes the JSON branch here too
    let fields = if content_type.contains("multipart/form-data") {
        fields::multipart(request.body)?
    } else {
        fields::json(request.body)?
    };
    if fields.file.is_none() && fields.file_url.is_none() && fields.file_id.is_none() {
        return Err(IngestRequestError::MissingSource);
    }
    let file = fields
        .file
        .map(|upload| secure_uploaded_file(&upload, max_upload_bytes, scanner, new_id))
        .transpose()?;
    if !fields.ingest_options.contains_key("vector_store") {
        return Err(IngestRequestError::MissingVectorStore);
    }
    credentials::check(&fields.ingest_options)?;
    Ok(ParsedIngestRequest {
        ingest_options: fields.ingest_options,
        file,
        file_url: fields.file_url,
        file_id: fields.file_id,
    })
}

fn secure_uploaded_file<Id>(
    file: &UploadFile,
    max_upload_bytes: usize,
    scanner: &dyn MalwareScanner,
    new_id: Id,
) -> Result<SecuredDocument, IngestRequestError>
where
    Id: FnOnce() -> Uuid,
{
    // endpoints.py:438 reads at most MAX_UPLOAD_SIZE_BYTES + 1, so an oversized upload is sized by what was read, not by the body
    let content = file
        .content
        .get(..max_upload_bytes.saturating_add(1))
        .unwrap_or(file.content.as_slice());
    match validate_upload(content, scanner, new_id, max_upload_bytes) {
        UploadValidation::Secured(secured) => Ok(SecuredDocument {
            safe_filename: secured.safe_filename,
            content: content.to_vec(),
            content_type: secured.content_type,
        }),
        UploadValidation::Rejected(upload) => Err(IngestRequestError::UploadRejected { upload }),
    }
}
