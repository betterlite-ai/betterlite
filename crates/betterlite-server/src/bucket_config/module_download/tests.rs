#![cfg(test)]

use std::collections::BTreeMap;
use std::fs::{create_dir_all, read, write};
use std::io;
use std::path::Path;
use std::str;
use std::sync::{Arc, Mutex};

use tokio::runtime::Builder;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt::MakeWriter;

use super::{BucketModuleDownloader, ModuleDownloadError};
use crate::bucket_config::source::BucketType;
use crate::bucket_config::test_support::{
    FakeS3, MemoryOpener, credential_environment, environment_without_keys,
};

const MODULE: &str = "def handler():\n    return 1\n";

fn downloader(objects: BTreeMap<String, Vec<u8>>) -> BucketModuleDownloader {
    BucketModuleDownloader {
        environment: credential_environment(),
        s3: Arc::new(FakeS3::default()),
        gcs: Arc::new(MemoryOpener { objects }),
    }
}

fn module_objects() -> BTreeMap<String, Vec<u8>> {
    BTreeMap::from([("module.py".to_owned(), MODULE.as_bytes().to_vec())])
}

fn s3_downloader(
    transport: Arc<FakeS3>,
    environment: BTreeMap<String, String>,
) -> BucketModuleDownloader {
    BucketModuleDownloader {
        environment,
        s3: transport,
        gcs: Arc::new(MemoryOpener {
            objects: module_objects(),
        }),
    }
}

#[tokio::test]
async fn a_gcs_module_is_written_into_an_existing_directory() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("handler.py");

    downloader(module_objects())
        .download_module(BucketType::Gcs, "bucket", "module.py", &path)
        .await
        .unwrap();

    assert_eq!(read(&path).unwrap(), MODULE.as_bytes());
}

#[tokio::test]
async fn a_module_with_non_ascii_text_is_written_as_utf8() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("handler.py");
    let text = "def handler():\n    return \"h\u{e9}llo\"\n";
    let objects = BTreeMap::from([("module.py".to_owned(), text.as_bytes().to_vec())]);

    downloader(objects)
        .download_module(BucketType::Gcs, "bucket", "module.py", &path)
        .await
        .unwrap();

    assert_eq!(read(&path).unwrap(), text.as_bytes());
}

#[tokio::test]
async fn a_gcs_module_creates_the_parent_folders_of_the_local_path() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("callbacks/remote/handler.py");

    downloader(module_objects())
        .download_module(BucketType::Gcs, "bucket", "module.py", &path)
        .await
        .unwrap();

    assert_eq!(read(&path).unwrap(), MODULE.as_bytes());
}

#[tokio::test]
async fn an_s3_module_is_signed_and_written_to_the_local_path() {
    let transport = FakeS3::answering(200, MODULE.as_bytes());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("handler.py");

    s3_downloader(Arc::clone(&transport), credential_environment())
        .download_module(BucketType::S3, "bucket", "loggers/module.py", &path)
        .await
        .unwrap();

    assert_eq!(read(&path).unwrap(), MODULE.as_bytes());
    assert_eq!(
        transport.urls(),
        vec!["https://bucket.s3.us-east-1.amazonaws.com/loggers/module.py".to_owned()]
    );
    assert!(
        transport
            .headers()
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("authorization"))
    );
}

#[tokio::test]
async fn each_s3_download_signs_its_own_request() {
    let transport = FakeS3::answering(200, MODULE.as_bytes());
    transport.push_answer(200, MODULE.as_bytes());
    let downloader = s3_downloader(Arc::clone(&transport), credential_environment());
    let directory = tempfile::tempdir().unwrap();

    downloader
        .download_module(
            BucketType::S3,
            "bucket",
            "loggers/module.py",
            &directory.path().join("first.py"),
        )
        .await
        .unwrap();
    downloader
        .download_module(
            BucketType::S3,
            "bucket",
            "loggers/module.py",
            &directory.path().join("second.py"),
        )
        .await
        .unwrap();

    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requests.iter().all(|request| request
            .headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("authorization"))),
        "each request is signed: {requests:?}"
    );
}

#[tokio::test]
async fn an_empty_path_is_not_a_directory_to_create() {
    let error = downloader(module_objects())
        .download_module(BucketType::Gcs, "bucket", "module.py", Path::new(""))
        .await
        .unwrap_err();

    assert!(matches!(error, ModuleDownloadError::Write { .. }));
}

#[tokio::test]
async fn a_missing_gcs_object_leaves_no_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("handler.py");

    let error = downloader(BTreeMap::new())
        .download_module(BucketType::Gcs, "bucket", "module.py", &path)
        .await
        .unwrap_err();

    assert!(matches!(error, ModuleDownloadError::Read { .. }));
    assert!(!path.exists());
}

#[tokio::test]
async fn a_missing_s3_object_leaves_no_file() {
    let downloader = s3_downloader(
        FakeS3::answering(404, b"no such key"),
        credential_environment(),
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("handler.py");

    let error = downloader
        .download_module(BucketType::S3, "bucket", "module.py", &path)
        .await
        .unwrap_err();

    assert!(matches!(error, ModuleDownloadError::Read { .. }));
    assert!(!path.exists());
}

#[tokio::test]
async fn an_s3_download_without_resolvable_credentials_is_the_read_error() {
    let transport = FakeS3::answering(200, MODULE.as_bytes());
    let downloader = s3_downloader(Arc::clone(&transport), environment_without_keys());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("handler.py");

    let error = downloader
        .download_module(BucketType::S3, "bucket", "module.py", &path)
        .await
        .unwrap_err();

    assert!(matches!(error, ModuleDownloadError::Read { .. }));
    assert!(transport.urls().is_empty());
    assert!(!path.exists());
}

#[tokio::test]
async fn a_gcs_object_that_is_not_utf8_leaves_no_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("handler.py");
    let objects = BTreeMap::from([("module.py".to_owned(), vec![0xff, 0xfe, 0x00])]);

    let error = downloader(objects)
        .download_module(BucketType::Gcs, "bucket", "module.py", &path)
        .await
        .unwrap_err();

    assert!(matches!(error, ModuleDownloadError::Decode { .. }));
    assert!(!path.exists());
}

#[tokio::test]
async fn an_s3_object_that_is_not_utf8_is_the_decode_error() {
    let downloader = s3_downloader(
        FakeS3::answering(200, &[0xff, 0xfe, 0x00]),
        credential_environment(),
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("handler.py");

    let error = downloader
        .download_module(BucketType::S3, "bucket", "module.py", &path)
        .await
        .unwrap_err();

    assert!(matches!(error, ModuleDownloadError::Decode { .. }));
    assert!(!path.exists());
}

#[tokio::test]
async fn a_parent_path_that_is_a_file_is_the_directory_error() {
    let directory = tempfile::tempdir().unwrap();
    let blocker = directory.path().join("blocker");
    write(&blocker, b"not a directory").unwrap();
    let path = blocker.join("handler.py");

    let error = downloader(module_objects())
        .download_module(BucketType::Gcs, "bucket", "module.py", &path)
        .await
        .unwrap_err();

    assert!(matches!(error, ModuleDownloadError::Directories { .. }));
}

#[tokio::test]
async fn a_target_that_is_a_directory_is_the_write_error() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("handler.py");
    create_dir_all(&path).unwrap();

    let error = downloader(module_objects())
        .download_module(BucketType::Gcs, "bucket", "module.py", &path)
        .await
        .unwrap_err();

    assert!(matches!(error, ModuleDownloadError::Write { .. }));
}

#[test]
fn the_module_text_is_not_logged() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("handler.py");
    let records = Arc::new(Mutex::new(String::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(CaptureWriter(Arc::clone(&records)))
        .finish();
    let downloader = downloader(module_objects());

    with_default(subscriber, || {
        Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(downloader.download_module(BucketType::Gcs, "bucket", "module.py", &path))
            .unwrap();
    });

    let captured = records.lock().unwrap().clone();
    assert!(
        captured.contains("handler.py"),
        "the path is logged: {captured}"
    );
    assert!(
        !captured.contains("return 1"),
        "the module text is logged: {captured}"
    );
}

#[derive(Clone)]
struct CaptureWriter(Arc<Mutex<String>>);

impl io::Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = str::from_utf8(buf)
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
        self.0
            .lock()
            .map_err(|source| io::Error::other(source.to_string()))?
            .push_str(text);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for CaptureWriter {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}
