use thiserror::Error;

use crate::api::request_form::values::FormError;
use crate::rag::upload_security::RejectedUpload;

use super::base64::Base64DecodeError;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IngestRequestError {
    #[error("Invalid base64 content: {source}")]
    InvalidBase64 { source: Base64DecodeError },
    #[error("Must provide file, file_url, or file_id")]
    MissingSource,
    #[error("{}", .upload.message)]
    UploadRejected { upload: RejectedUpload },
    #[error("ingest_options must contain 'vector_store' configuration")]
    MissingVectorStore,
    #[error(
        "'{field}' cannot be set in ingest_options.vector_store. Credentials must be configured server-side."
    )]
    BlockedCredential { field: &'static str },
    #[error("'{field}' must be a JSON object")]
    NotAnObject { field: &'static str },
    #[error("'{field}' must be a string")]
    NotText { field: &'static str },
    #[error("the multipart 'request' field is not valid JSON")]
    RequestInvalidJson,
    #[error("the multipart 'request' field must be a JSON object")]
    RequestNotObject,
    #[error("the multipart 'request' field must be text")]
    RequestNotText,
    #[error("the request body does not match the content type")]
    BodyKindMismatch,
    #[error("{0}")]
    Form(#[from] FormError),
}

impl IngestRequestError {
    // every rejection the decoder raises is the 400 HTTPException of _secure_uploaded_file and parse_rag_ingest_request
    pub const STATUS: u16 = 400;
}
