#![cfg(test)]

use std::collections::BTreeMap;
use std::fs::read;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use betterlite_pyhost::instance_loader::{InstanceLoader, LoadError};
use betterlite_pyhost::instance_ref::Storage;
use betterlite_pyhost::json_values::JsonObject;
use betterlite_pyhost::remote_module::{ModuleFetcher, ModuleRequest};
use serde_json::json;

use super::{BucketFetchError, BucketModuleFetcher};
use crate::python_hooks::test_support;

const MODULE: &str = "def handler():\n    return \"remote\"\n";

fn request(storage: Storage, object_key: &str, local_path: PathBuf) -> ModuleRequest {
    ModuleRequest {
        storage,
        bucket: "bucket".to_owned(),
        object_key: object_key.to_owned(),
        local_path,
    }
}

fn gcs_objects() -> BTreeMap<String, Vec<u8>> {
    BTreeMap::from([("module.py".to_owned(), MODULE.as_bytes().to_vec())])
}

#[test]
fn a_gcs_module_is_downloaded_to_the_local_path() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("module.py");
    let downloader = Arc::new(test_support::gcs_downloader(gcs_objects()));
    let fetcher = BucketModuleFetcher::supplied(downloader);
    fetcher
        .fetch(&request(Storage::Gcs, "module.py", path.clone()))
        .unwrap();
    assert_eq!(read(&path).unwrap(), MODULE.as_bytes());
}

#[test]
fn an_s3_module_is_downloaded_under_its_object_key() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("logging.py");
    let (downloader, transport) = test_support::s3_downloader(MODULE.as_bytes());
    let fetcher = BucketModuleFetcher::supplied(Arc::new(downloader));
    fetcher
        .fetch(&request(Storage::S3, "loggers/logging.py", path.clone()))
        .unwrap();
    assert_eq!(read(&path).unwrap(), MODULE.as_bytes());
    assert_eq!(
        transport.urls(),
        vec!["https://bucket.s3.us-east-1.amazonaws.com/loggers/logging.py".to_owned()]
    );
}

#[test]
fn the_production_fetcher_reads_the_proxy_settings_from_its_environment() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("module.py");
    let environment = BTreeMap::from([(
        "DISABLE_AIOHTTP_TRUST_ENV".to_owned(),
        "not-a-flag".to_owned(),
    )]);
    let fetcher = BucketModuleFetcher::production(environment);
    let error = fetcher
        .fetch(&request(Storage::Gcs, "module.py", path))
        .unwrap_err();
    assert!(
        matches!(&error, BucketFetchError::Proxy { .. }),
        "got: {error:?}"
    );
}

#[tokio::test]
async fn a_download_failure_reads_like_litellms_remote_load_error() {
    let workspace = test_support::workspace();
    let service = test_support::service(Some(workspace.config_path.clone()));
    let runtime = service
        .inner
        .get_or_try_init(|| service.start_runtime())
        .await
        .unwrap();
    let downloader = Arc::new(test_support::gcs_downloader(BTreeMap::new()));
    let fetcher: Arc<dyn ModuleFetcher<Error = BucketFetchError>> =
        Arc::new(BucketModuleFetcher::supplied(downloader));
    let loader = InstanceLoader::new(runtime.host.clone(), fetcher);
    let url = "gcs://bucket/module.handler";
    let error = loader
        .load(&runtime.tracker, url, Some(workspace.config_path.as_path()))
        .await
        .unwrap_err();
    assert!(matches!(&error, LoadError::Remote { .. }), "got: {error:?}");
    let text = error.to_string();
    assert!(
        text.contains("Failed to load custom logger from gcs://bucket/module.handler"),
        "got: {text}"
    );
    assert!(
        text.contains("Failed to download module.py from gcs bucket bucket"),
        "got: {text}"
    );
}

#[tokio::test]
async fn a_fetched_module_is_imported_by_the_instance_loader() {
    let workspace = test_support::workspace();
    let service = test_support::service(Some(workspace.config_path.clone()));
    let runtime = service
        .inner
        .get_or_try_init(|| service.start_runtime())
        .await
        .unwrap();
    let downloader = Arc::new(test_support::gcs_downloader(gcs_objects()));
    let fetcher: Arc<dyn ModuleFetcher<Error = BucketFetchError>> =
        Arc::new(BucketModuleFetcher::supplied(downloader));
    let loader = InstanceLoader::new(runtime.host.clone(), fetcher);
    let url = "gcs://bucket/module.handler";
    let loaded = loader
        .load(&runtime.tracker, url, Some(workspace.config_path.as_path()))
        .await
        .unwrap();
    assert_eq!(loaded.value, url);
    let handle = loaded.handle(&runtime.host).unwrap();
    let result = handle
        .call(
            &runtime.host,
            &runtime.tracker,
            Duration::from_secs(5),
            Vec::new(),
            JsonObject::new(),
        )
        .await
        .unwrap();
    assert_eq!(result, json!("remote"));
}
