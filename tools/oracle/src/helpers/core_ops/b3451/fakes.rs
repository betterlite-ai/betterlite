use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_config::credentials::CredentialValues;
use betterlite_core::rag::ingestion::credentials::{CredentialLookup, CredentialLookupError};
use betterlite_core::rag::ingestion::ocr::{OcrError, OcrOperation, OcrRequest, OcrResponse};
use betterlite_core::rag::ingestion::upload::{FetchedResponse, UrlFetch, UrlFetchError};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError};
use http::HeaderMap;
use http::HeaderValue;
use http::header::CONTENT_TYPE;

use super::inputs::FetchInput;

#[derive(Default)]
pub(super) struct RecordingOcr {
    pub(super) calls: Mutex<Vec<OcrRequest>>,
    pages: Vec<String>,
}

impl RecordingOcr {
    #[must_use]
    pub(super) fn with_pages(pages: &[String]) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            pages: pages.to_vec(),
        }
    }
}

#[async_trait]
impl OcrOperation for RecordingOcr {
    async fn aocr(&self, request: OcrRequest) -> Result<OcrResponse, OcrError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        Ok(OcrResponse::from_pages(self.pages.clone()))
    }
}

pub(super) struct RecordingFetch {
    status: u16,
    content_type: Option<String>,
    body: Vec<u8>,
}

impl RecordingFetch {
    pub(super) fn answer(input: Option<&FetchInput>) -> Result<Self, Box<HelperError>> {
        let Some(fetch) = input else {
            return Ok(Self {
                status: 200,
                content_type: None,
                body: Vec::new(),
            });
        };
        Ok(Self {
            status: fetch.status,
            content_type: fetch.content_type.clone(),
            body: BASE64.decode(&fetch.body_base64).or_value_error()?,
        })
    }

    fn headers(&self) -> Result<HeaderMap, UrlFetchError> {
        let Some(content_type) = &self.content_type else {
            return Ok(HeaderMap::new());
        };
        let value =
            HeaderValue::from_str(content_type).map_err(|_error| UrlFetchError::Failed {
                message: "the fake content type is not a header value".to_owned(),
            })?;
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, value);
        Ok(headers)
    }
}

#[async_trait]
impl UrlFetch for RecordingFetch {
    async fn fetch(&self, _url: &str) -> Result<FetchedResponse, UrlFetchError> {
        Ok(FetchedResponse {
            status: self.status,
            headers: self.headers()?,
            body: self.body.clone(),
        })
    }
}

pub(super) struct FixedCredentials {
    pub(super) values: BTreeMap<String, CredentialValues>,
}

impl CredentialLookup for FixedCredentials {
    fn credential_values(
        &self,
        credential_name: &str,
    ) -> Result<CredentialValues, CredentialLookupError> {
        self.values
            .get(credential_name)
            .cloned()
            .ok_or_else(|| CredentialLookupError::NotFound {
                credential_name: credential_name.to_owned(),
            })
    }
}
