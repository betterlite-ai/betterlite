use crate::ocr_config::OCR_INLINE_MAX_BYTES;
use crate::ocr_config::connection::OcrConnection;
use crate::ocr_error::{OcrError, OcrTransportError};
use crate::ocr_http::OcrClient;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_http::proxy::StreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::user_url::error::{SsrfError, UserUrlError};
use data_url::{DataUrl, DataUrlError, forgiving_base64::DecodeError};
use http::HeaderMap;
use http::header::CONTENT_TYPE;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use url::Url;
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum OcrDocument {
    #[serde(rename = "document_url")]
    DocumentUrl {
        document_url: String,
        #[serde(flatten)]
        extra: BTreeMap<String, Option<String>>,
    },
    #[serde(rename = "image_url")]
    ImageUrl {
        image_url: String,
        #[serde(flatten)]
        extra: BTreeMap<String, Option<String>>,
    },
}
impl OcrDocument {
    #[must_use]
    pub fn source(&self) -> &str {
        match self {
            Self::DocumentUrl { document_url, .. } => document_url,
            Self::ImageUrl { image_url, .. } => image_url,
        }
    }
    #[must_use]
    pub fn with_source(self, source: String) -> Self {
        match self {
            Self::DocumentUrl { extra, .. } => Self::DocumentUrl {
                document_url: source,
                extra,
            },
            Self::ImageUrl { extra, .. } => Self::ImageUrl {
                image_url: source,
                extra,
            },
        }
    }
}
pub struct InlineDocument<'source>(pub DataUrl<'source>);
impl<'source> InlineDocument<'source> {
    pub fn parse(source: &'source str) -> Result<Option<Self>, OcrError> {
        match DataUrl::process(source) {
            Ok(url) => Ok(Some(Self(url))),
            Err(DataUrlError::NotADataUrl) => Ok(None),
            Err(DataUrlError::NoComma) => Err(OcrError::InvalidDataUri),
        }
    }
    pub fn decode(&self, max_bytes: usize) -> Result<Vec<u8>, OcrError> {
        let mut body = Vec::new();
        self.0
            .decode(|bytes| {
                if bytes.len() > max_bytes.saturating_sub(body.len()) {
                    return Err(OcrError::InlineDocumentTooLarge);
                }
                body.extend_from_slice(bytes);
                Ok(())
            })
            .map_err(|failure| match failure {
                DecodeError::InvalidBase64(_) => OcrError::InvalidDataUri,
                DecodeError::WriteError(written) => written,
            })?;
        Ok(body)
    }
}
pub fn validate_inline_document(document: &OcrDocument) -> Result<(), OcrError> {
    let inline = InlineDocument::parse(document.source())?.ok_or(OcrError::InvalidDataUri)?;
    inline.decode(OCR_INLINE_MAX_BYTES)?;
    Ok(())
}
pub async fn inline_remote_document(
    client: &OcrClient,
    document: OcrDocument,
    connection: &OcrConnection,
) -> Result<OcrDocument, OcrError> {
    let source = document.source();
    if !source.starts_with("http://") && !source.starts_with("https://") {
        validate_inline_document(&document)?;
        return Ok(document);
    }
    let limit = connection.settings.max_download_bytes;
    if limit == 0 {
        return Err(OcrError::DownloadDisabled);
    }
    Url::parse(source).map_err(|_source| OcrError::RequestField {
        path: "document URL".to_owned(),
    })?;
    let response = client
        .document_fetcher
        .fetch(&client.document_http, source, &HeaderMap::new())
        .await
        .map_err(document_fetch_error)?;
    let status = response.response.status();
    if !status.is_success() {
        return Err(OcrError::Transport(OcrTransportError::Http {
            status: status.as_u16(),
            body: "the OCR document download failed".to_owned(),
        }));
    }
    let content_type = content_type(&response);
    let bytes = download_body(response, limit).await?;
    let converted = document.with_source(format!(
        "data:{content_type};base64,{}",
        STANDARD.encode(bytes)
    ));
    validate_inline_document(&converted)?;
    Ok(converted)
}
fn content_type(response: &StreamResponse) -> String {
    response
        .response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("application/octet-stream")
        .to_owned()
}
async fn download_body(mut response: StreamResponse, limit: u64) -> Result<Vec<u8>, OcrError> {
    if response
        .response
        .content_length()
        .is_some_and(|declared| declared > limit)
    {
        return Err(OcrError::DownloadTooLarge);
    }
    let limit_bytes = usize::try_from(limit).unwrap_or(usize::MAX);
    let mut body = Vec::new();
    while let Some(chunk) = response.response.chunk().await.map_err(|error| {
        if error.is_timeout() {
            return OcrError::Transport(OcrTransportError::Timeout);
        }
        OcrError::Transport(OcrTransportError::Failed(TransportError::Body(error)))
    })? {
        if body
            .len()
            .checked_add(chunk.len())
            .is_none_or(|total| total > limit_bytes)
        {
            return Err(OcrError::DownloadTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}
fn document_fetch_error(failure: UserUrlError) -> OcrError {
    match failure {
        UserUrlError::Ssrf(SsrfError::TooManyRedirects) | UserUrlError::TooManyRedirects => {
            OcrError::TooManyRedirects
        }
        UserUrlError::Ssrf(SsrfError::NoLocation) => OcrError::MissingRedirectLocation,
        UserUrlError::Ssrf(_) => OcrError::BlockedDocumentUrl,
        UserUrlError::Unrequestable { .. } => OcrError::RequestField {
            path: "document URL".to_owned(),
        },
        UserUrlError::Parse(_) | UserUrlError::InvalidUrl(_) | UserUrlError::LocationHeader(_) => {
            OcrError::InvalidRedirect
        }
        UserUrlError::Transport(
            TransportError::Deadline(_) | TransportError::ConnectTimeout(_),
        ) => OcrError::Transport(OcrTransportError::Timeout),
        UserUrlError::Transport(error) => OcrError::Transport(OcrTransportError::Failed(error)),
    }
}
mod tests;
