#![cfg(test)]

use std::collections::BTreeMap;
use std::fs::write;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use tempfile::TempDir;
use tracing::subscriber::{DefaultGuard, set_default};

use super::PythonHooks;
use crate::bucket_config::gcs_reader::GcsBucketOpener;
use crate::bucket_config::module_download::BucketModuleDownloader;
use crate::bucket_config::object::BucketObjectReader;
use crate::bucket_config::s3_reader::{S3Get, S3GetRequest, S3GetResponse, S3TransportFailure};

pub(crate) const SHUTDOWN_GRACE: Duration = Duration::from_millis(100);

pub(crate) fn service(config_file_path: Option<PathBuf>) -> PythonHooks {
    let mut service = PythonHooks::new(config_file_path);
    service.shutdown_grace = SHUTDOWN_GRACE;
    service
}

pub(crate) struct Workspace {
    pub(crate) directory: TempDir,
    pub(crate) config_path: PathBuf,
}

impl Workspace {
    pub(crate) fn module(&self, name: &str, source: &str) -> String {
        let path = self.directory.path().join(format!("{name}.py"));
        write(&path, source).unwrap();
        name.to_owned()
    }
}

pub(crate) fn workspace() -> Workspace {
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("config.yaml");
    write(&config_path, b"model_list: []\n").unwrap();
    Workspace {
        directory,
        config_path,
    }
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

#[derive(Debug)]
pub(crate) struct FakeS3 {
    seen: Mutex<Vec<S3GetRequest>>,
    body: Vec<u8>,
}

impl FakeS3 {
    pub(crate) fn new(body: &[u8]) -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
            body: body.to_vec(),
        })
    }

    pub(crate) fn urls(&self) -> Vec<String> {
        let seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        let mut urls = Vec::with_capacity(seen.len());
        for request in seen.iter() {
            urls.push(request.url.clone());
        }
        drop(seen);
        urls
    }
}

#[async_trait]
impl S3Get for FakeS3 {
    async fn get(&self, request: S3GetRequest) -> Result<S3GetResponse, S3TransportFailure> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        Ok(S3GetResponse {
            status: 200,
            body: self.body.clone(),
        })
    }
}

pub(crate) fn gcs_downloader(objects: BTreeMap<String, Vec<u8>>) -> BucketModuleDownloader {
    BucketModuleDownloader {
        environment: BTreeMap::new(),
        s3: FakeS3::new(b""),
        gcs: Arc::new(MemoryOpener { objects }),
    }
}

pub(crate) fn s3_downloader(body: &[u8]) -> (BucketModuleDownloader, Arc<FakeS3>) {
    let transport = FakeS3::new(body);
    let s3: Arc<dyn S3Get> = Arc::<FakeS3>::clone(&transport);
    let downloader = BucketModuleDownloader {
        environment: credential_environment(),
        s3,
        gcs: Arc::new(MemoryOpener {
            objects: BTreeMap::new(),
        }),
    };
    (downloader, transport)
}

pub(crate) fn credential_environment() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("AWS_ACCESS_KEY_ID".to_owned(), "AKIDEXAMPLE".to_owned()),
        ("AWS_SECRET_ACCESS_KEY".to_owned(), "secret".to_owned()),
        ("AWS_EC2_METADATA_DISABLED".to_owned(), "true".to_owned()),
        (
            "AWS_CONFIG_FILE".to_owned(),
            "/nonexistent/betterlite-b165-config".to_owned(),
        ),
        (
            "AWS_SHARED_CREDENTIALS_FILE".to_owned(),
            "/nonexistent/betterlite-b165-credentials".to_owned(),
        ),
    ])
}

#[derive(Clone, Default)]
pub(crate) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(crate) fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).unwrap()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}
