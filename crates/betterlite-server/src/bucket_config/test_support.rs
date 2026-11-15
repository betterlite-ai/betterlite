#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;

use super::gcs_reader::GcsBucketOpener;
use super::object::BucketObjectReader;
use super::s3_reader::{S3Get, S3GetRequest, S3GetResponse, S3TransportFailure};

pub(crate) const ACCESS_KEY: &str = "AKIDEXAMPLE";
pub(crate) const SECRET_KEY: &str = "secret";

pub(crate) fn credential_environment() -> BTreeMap<String, String> {
    let mut environment = environment_without_keys();
    environment.insert("AWS_ACCESS_KEY_ID".to_owned(), ACCESS_KEY.to_owned());
    environment.insert("AWS_SECRET_ACCESS_KEY".to_owned(), SECRET_KEY.to_owned());
    environment
}

pub(crate) fn environment_without_keys() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("AWS_EC2_METADATA_DISABLED".to_owned(), "true".to_owned()),
        (
            "AWS_CONFIG_FILE".to_owned(),
            "/nonexistent/betterlite-b4143-config".to_owned(),
        ),
        (
            "AWS_SHARED_CREDENTIALS_FILE".to_owned(),
            "/nonexistent/betterlite-b4143-credentials".to_owned(),
        ),
    ])
}

pub(crate) struct MemoryReader {
    pub(crate) objects: BTreeMap<String, Vec<u8>>,
}

#[async_trait]
impl BucketObjectReader for MemoryReader {
    async fn read_object(&self, object_key: &str) -> Option<Vec<u8>> {
        self.objects.get(object_key).cloned()
    }
}

pub(crate) struct MemoryOpener {
    pub(crate) objects: BTreeMap<String, Vec<u8>>,
}

impl GcsBucketOpener for MemoryOpener {
    fn open(&self, _bucket_name: &str) -> Box<dyn BucketObjectReader> {
        Box::new(MemoryReader {
            objects: self.objects.clone(),
        })
    }
}

#[derive(Debug, Default)]
pub(crate) struct FakeS3 {
    answers: Mutex<VecDeque<Result<S3GetResponse, S3TransportFailure>>>,
    seen: Mutex<Vec<S3GetRequest>>,
}

impl FakeS3 {
    pub(crate) fn answering(status: u16, body: &[u8]) -> Arc<Self> {
        let fake = Self::default();
        fake.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(Ok(S3GetResponse {
                status,
                body: body.to_vec(),
            }));
        Arc::new(fake)
    }

    pub(crate) fn failing(error: S3TransportFailure) -> Arc<Self> {
        let fake = Self::default();
        fake.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(Err(error));
        Arc::new(fake)
    }

    pub(crate) fn push_answer(&self, status: u16, body: &[u8]) {
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(Ok(S3GetResponse {
                status,
                body: body.to_vec(),
            }));
    }

    pub(crate) fn requests(&self) -> Vec<S3GetRequest> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn urls(&self) -> Vec<String> {
        let mut urls = Vec::new();
        for request in self.requests() {
            urls.push(request.url);
        }
        urls
    }

    pub(crate) fn headers(&self) -> Vec<(String, String)> {
        self.requests()
            .first()
            .map_or_else(Vec::new, |request| request.headers.clone())
    }
}

#[async_trait]
impl S3Get for FakeS3 {
    async fn get(&self, request: S3GetRequest) -> Result<S3GetResponse, S3TransportFailure> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Ok(S3GetResponse {
                status: 200,
                body: Vec::new(),
            }))
    }
}
