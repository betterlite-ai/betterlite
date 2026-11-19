#![cfg(test)]

#[path = "tests/chunk.rs"]
mod chunk;
#[path = "tests/chunk_config.rs"]
mod chunk_config;
#[path = "tests/credentials.rs"]
mod credentials;
#[path = "tests/init.rs"]
mod init;
#[path = "tests/ocr.rs"]
mod ocr;
#[path = "tests/store.rs"]
mod store;
#[path = "tests/upload.rs"]
mod upload;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_config::credentials::CredentialValues;
use http::HeaderMap;
use serde_json::value::RawValue;

use super::config::IngestionOptions;
use super::credentials::{CredentialLookup, CredentialLookupError};
use super::ocr::{OcrError, OcrOperation, OcrRequest, OcrResponse};
use super::upload::{FetchedResponse, UrlFetch, UrlFetchError};
use super::{IngestionPorts, IngestionState};

#[derive(Default)]
struct FakeOcr {
    calls: Mutex<Vec<OcrRequest>>,
    pages: Vec<String>,
    failure: Option<String>,
}

impl FakeOcr {
    #[must_use]
    fn returning(pages: Vec<String>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            pages,
            failure: None,
        }
    }

    #[must_use]
    fn failing(message: &str) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            pages: Vec::new(),
            failure: Some(message.to_owned()),
        }
    }
}

#[async_trait]
impl OcrOperation for FakeOcr {
    async fn aocr(&self, request: OcrRequest) -> Result<OcrResponse, OcrError> {
        self.calls.lock().unwrap().push(request);
        if let Some(message) = &self.failure {
            return Err(OcrError::Failed {
                message: message.clone(),
            });
        }
        Ok(OcrResponse::from_pages(self.pages.clone()))
    }
}

struct FakeFetch {
    calls: Mutex<Vec<String>>,
    status: u16,
    headers: HeaderMap,
    body: Vec<u8>,
    failure: Option<String>,
}

impl Default for FakeFetch {
    fn default() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            status: 200,
            headers: HeaderMap::new(),
            body: Vec::new(),
            failure: None,
        }
    }
}

impl FakeFetch {
    #[must_use]
    fn reply(mut self, status: u16, headers: HeaderMap, body: &[u8]) -> Self {
        self.status = status;
        self.headers = headers;
        self.body = body.to_vec();
        self
    }

    #[must_use]
    fn failing(mut self, message: &str) -> Self {
        self.failure = Some(message.to_owned());
        self
    }
}

#[async_trait]
impl UrlFetch for FakeFetch {
    async fn fetch(&self, url: &str) -> Result<FetchedResponse, UrlFetchError> {
        self.calls.lock().unwrap().push(url.to_owned());
        if let Some(message) = &self.failure {
            return Err(UrlFetchError::Failed {
                message: message.clone(),
            });
        }
        Ok(FetchedResponse {
            status: self.status,
            headers: self.headers.clone(),
            body: self.body.clone(),
        })
    }
}

#[derive(Default)]
struct FakeCredentials {
    values: Option<CredentialValues>,
}

impl CredentialLookup for FakeCredentials {
    fn credential_values(
        &self,
        credential_name: &str,
    ) -> Result<CredentialValues, CredentialLookupError> {
        self.values.as_ref().map_or_else(
            || {
                Err(CredentialLookupError::NotFound {
                    credential_name: credential_name.to_owned(),
                })
            },
            |values| Ok(values.clone()),
        )
    }
}

struct Harness {
    ocr: Arc<FakeOcr>,
    fetch: Arc<FakeFetch>,
    credentials: Arc<FakeCredentials>,
}

impl Default for Harness {
    fn default() -> Self {
        Self {
            ocr: Arc::new(FakeOcr::default()),
            fetch: Arc::new(FakeFetch::default()),
            credentials: Arc::new(FakeCredentials::default()),
        }
    }
}

impl Harness {
    #[must_use]
    fn with_ocr(mut self, ocr: FakeOcr) -> Self {
        self.ocr = Arc::new(ocr);
        self
    }

    #[must_use]
    fn with_fetch(mut self, fetch: FakeFetch) -> Self {
        self.fetch = Arc::new(fetch);
        self
    }

    #[must_use]
    fn with_credentials(mut self, values: &str) -> Self {
        self.credentials = Arc::new(FakeCredentials {
            values: Some(serde_json::from_str(values).expect("the credential values parse")),
        });
        self
    }

    fn state(&self, options: IngestionOptions) -> IngestionState {
        self.routed(options, None)
    }

    fn routed(
        &self,
        options: IngestionOptions,
        router: Option<Arc<dyn OcrOperation>>,
    ) -> IngestionState {
        let ocr: Arc<dyn OcrOperation> = Arc::<FakeOcr>::clone(&self.ocr);
        let uploader: Arc<dyn UrlFetch> = Arc::<FakeFetch>::clone(&self.fetch);
        let credentials: Arc<dyn CredentialLookup> =
            Arc::<FakeCredentials>::clone(&self.credentials);
        IngestionState::new(
            options,
            router,
            IngestionPorts {
                ocr,
                uploader,
                credentials,
            },
        )
        .expect("the state builds")
    }
}

fn text_of(config: &BTreeMap<String, Box<RawValue>>, key: &str) -> String {
    let raw = config.get(key).expect("the key");
    serde_json::from_str(raw.get()).expect("a string")
}
