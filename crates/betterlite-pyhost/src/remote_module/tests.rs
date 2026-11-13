#![cfg(test)]

use std::error::Error as _;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};

use pyo3::types::PyAnyMethods as _;
use tracing::subscriber::set_global_default;

use super::{FetchError, ModuleFetcher, ModuleRequest, RemoteError};
use crate::host::{DEFAULT_SHUTDOWN_GRACE, HostConfig, PythonHost};
use crate::instance_loader::{InstanceLoader, LoadError};
use crate::instance_ref::Storage;

struct Recorder {
    body: Option<&'static str>,
    failure: bool,
    requests: Mutex<Vec<ModuleRequest>>,
}

impl Recorder {
    fn new(body: Option<&'static str>, failure: bool) -> Arc<Self> {
        Arc::new(Self {
            body,
            failure,
            requests: Mutex::new(Vec::new()),
        })
    }

    fn requests(&self) -> Vec<ModuleRequest> {
        match self.requests.lock() {
            Ok(requests) => requests.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

impl ModuleFetcher for Recorder {
    fn fetch(&self, request: &ModuleRequest) -> Result<(), FetchError> {
        match self.requests.lock() {
            Ok(mut requests) => requests.push(request.clone()),
            Err(poisoned) => poisoned.into_inner().push(request.clone()),
        }
        if self.failure {
            return Err(FetchError {
                message: "the bucket refused the read".to_owned(),
            });
        }
        let Some(body) = self.body else {
            return Ok(());
        };
        fs::write(&request.local_path, body).map_err(|error| FetchError {
            message: error.to_string(),
        })
    }
}

fn loader(fetcher: Arc<Recorder>) -> (PythonHost, InstanceLoader) {
    let config = HostConfig {
        module_dirs: Vec::new(),
        shutdown_grace: DEFAULT_SHUTDOWN_GRACE,
    };
    let host = PythonHost::start(&config).unwrap();
    let downloader: Arc<dyn ModuleFetcher> = fetcher;
    let loader = InstanceLoader::new(host.clone(), downloader);
    (host, loader)
}

fn config_path() -> &'static Path {
    Path::new("config.yaml")
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
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

fn capture_logs() -> Captured {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    // the load logs from the pool's blocking thread, so the subscriber has to be global
    set_global_default(subscriber).unwrap();
    captured
}

#[tokio::test]
async fn fetches_the_object_and_executes_it_under_the_module_path() {
    let recorder = Recorder::new(Some("name = __name__\ninstance = name\n"), false);
    let (host, loader) = loader(Arc::clone(&recorder));
    let instance = loader
        .load("s3://bucket/loggers/custom.instance", Some(config_path()))
        .await
        .unwrap();
    let text = host.attach(|py| instance.object.bind(py).extract::<String>().unwrap());
    assert_eq!(text, "loggers/custom");
    let requests = recorder.requests();
    let request = requests.first().unwrap();
    assert_eq!(request.storage, Storage::S3);
    assert_eq!(request.bucket, "bucket");
    assert_eq!(request.object_key, "loggers/custom.py");
    assert!(
        !request.local_path.exists(),
        "the downloaded file stays after the load"
    );
}

#[tokio::test]
async fn a_bucket_load_logs_its_start_and_its_success() {
    let captured = capture_logs();
    let recorder = Recorder::new(Some("instance = 1\n"), false);
    let (_, loader) = loader(Arc::clone(&recorder));
    loader
        .load("s3://bucket/loggers/custom.instance", Some(config_path()))
        .await
        .unwrap();
    let text = captured.text();
    assert!(
        text.contains("loading a custom logger module from a bucket"),
        "got: {text}"
    );
    assert!(
        text.contains("loaded a custom logger module from a bucket"),
        "got: {text}"
    );
}

#[tokio::test]
async fn a_gcs_url_uses_the_gcs_storage() {
    let recorder = Recorder::new(Some("instance = 1\n"), false);
    let (_, loader) = loader(Arc::clone(&recorder));
    loader
        .load("gcs://bucket/pkg/mod.instance", Some(config_path()))
        .await
        .unwrap();
    let requests = recorder.requests();
    let request = requests.first().unwrap();
    assert_eq!(request.storage, Storage::Gcs);
    assert_eq!(request.object_key, "pkg/mod.py");
}

#[tokio::test]
async fn a_failed_fetch_reads_as_the_download_error() {
    let recorder = Recorder::new(None, true);
    let (_, loader) = loader(Arc::clone(&recorder));
    let error = loader
        .load("s3://bucket/loggers/custom.instance", Some(config_path()))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Failed to load custom logger from s3://bucket/loggers/custom.instance: Failed to download loggers/custom.py from s3 bucket bucket"
    );
    let LoadError::Remote { url, source } = &error else {
        panic!("expected a remote error, got {error:?}");
    };
    assert_eq!(url, "s3://bucket/loggers/custom.instance");
    assert!(matches!(&**source, RemoteError::Fetch { .. }));
    let cause = source.source().unwrap();
    assert_eq!(cause.to_string(), "the bucket refused the read");
}

#[tokio::test]
async fn a_module_that_raises_is_wrapped_with_its_message() {
    let recorder = Recorder::new(Some("raise ValueError('boom')\n"), false);
    let (_, loader) = loader(Arc::clone(&recorder));
    let error = loader
        .load("s3://bucket/loggers/custom.instance", Some(config_path()))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Failed to load custom logger from s3://bucket/loggers/custom.instance: boom"
    );
}

#[tokio::test]
async fn a_missing_instance_is_wrapped_with_the_runtime_text() {
    let recorder = Recorder::new(Some("other = 1\n"), false);
    let (_, loader) = loader(Arc::clone(&recorder));
    let error = loader
        .load("s3://bucket/loggers/custom.instance", Some(config_path()))
        .await
        .unwrap_err();
    let text = error.to_string();
    assert!(
        text.starts_with("Failed to load custom logger from s3://bucket/loggers/custom.instance: "),
        "got: {text}"
    );
    assert!(text.contains("has no attribute 'instance'"), "got: {text}");
}

#[tokio::test]
async fn the_temp_file_is_removed_after_a_failing_module() {
    let recorder = Recorder::new(Some("raise ValueError('boom')\n"), false);
    let (_, loader) = loader(Arc::clone(&recorder));
    let error = loader
        .load("s3://bucket/loggers/custom.instance", Some(config_path()))
        .await
        .unwrap_err();
    let requests = recorder.requests();
    let request = requests.first().unwrap();
    assert!(error.to_string().contains("boom"));
    assert!(
        !request.local_path.exists(),
        "the temp file survives the failed load"
    );
}

#[tokio::test]
async fn a_remote_value_without_a_config_path_is_refused() {
    let recorder = Recorder::new(Some("instance = 1\n"), false);
    let (_, loader) = loader(Arc::clone(&recorder));
    let error = loader
        .load("s3://bucket/loggers/custom.instance", None)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Remote module loading (s3://, gcs://) is only permitted from the config-file load path. Register the module under ``litellm_settings`` in your config.yaml instead."
    );
    assert!(matches!(error, LoadError::RemoteNeedsConfigPath));
    assert!(recorder.requests().is_empty());
}

#[tokio::test]
async fn a_url_error_is_wrapped_before_any_download() {
    let recorder = Recorder::new(Some("instance = 1\n"), false);
    let (_, loader) = loader(Arc::clone(&recorder));
    let error = loader
        .load("s3://bucket-only", Some(config_path()))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Failed to load custom logger from s3://bucket-only: Invalid URL format: s3://bucket-only. Expected: s3://bucket-name/path/to/module.instance"
    );
    assert!(recorder.requests().is_empty());
}

#[tokio::test]
async fn a_py_extension_error_is_wrapped_before_any_download() {
    let recorder = Recorder::new(Some("instance = 1\n"), false);
    let (_, loader) = loader(Arc::clone(&recorder));
    let error = loader
        .load("gcs://bucket/mod.py", Some(config_path()))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Failed to load custom logger from gcs://bucket/mod.py: Invalid URL format in gcs://bucket/mod.py. Don't include '.py' extension and you must specify the instance name. Expected format: gcs://bucket/mod.instance_name (e.g., gcs://bucket/mod.proxy_handler_instance)"
    );
    assert!(recorder.requests().is_empty());
}
