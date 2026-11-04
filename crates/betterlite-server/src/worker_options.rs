use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

const TEMP_ROOT_ENV: [&str; 3] = ["TMPDIR", "TEMP", "TMP"];
#[cfg(unix)]
const DEFAULT_TEMP_ROOT: &str = "/tmp";
#[cfg(windows)]
const DEFAULT_TEMP_ROOT: &str = r"C:\TEMP";
#[cfg(not(any(unix, windows)))]
const DEFAULT_TEMP_ROOT: &str = ".";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    Hypercorn,
    Granian,
    Gunicorn,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum CallbackSetting {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LaunchOptions {
    pub backend: Backend,
    pub host: String,
    pub port: u16,
    pub workers: u32,
    pub runtime_threads: Option<u32>,
    pub certfile: Option<PathBuf>,
    pub keyfile: Option<PathBuf>,
    pub ciphers: Option<String>,
    pub max_requests: Option<u32>,
    pub max_requests_jitter: Option<u32>,
    pub metrics_port: Option<u16>,
    pub prometheus_dir: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TlsPlan {
    pub enabled: bool,
    pub ciphers: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProcessPlan {
    pub preload: bool,
    pub child_exit_cleanup: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LaunchPlan {
    pub backend: Backend,
    pub bind: String,
    pub workers: u32,
    pub timeout_seconds: Option<u32>,
    pub tls: TlsPlan,
    pub process: ProcessPlan,
    pub runtime_threads: Option<u32>,
    pub max_requests: Option<u32>,
    pub max_requests_jitter: Option<u32>,
    pub start_message: String,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DirectoryPreparation {
    pub path: PathBuf,
    pub action: String,
    pub should_prepare: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectoryMode {
    Plan,
    Prepare,
}

#[derive(Debug, thiserror::Error)]
pub enum DirectoryPreparationError {
    #[error("failed to prepare Prometheus multiprocess directory {path}: {source}")]
    Create {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum WorkerLaunchError {
    #[error("Both --ssl_certfile_path and --ssl_keyfile_path are required for SSL.")]
    IncompleteGranianTls,
    #[error("multi-worker and process-fork launch modes are unsupported by the native runtime")]
    UnsupportedRuntime,
}

struct BackendPlan {
    workers: u32,
    timeout_seconds: Option<u32>,
    ciphers: Option<String>,
    preload: bool,
    child_exit_cleanup: bool,
    runtime_threads: Option<u32>,
    max_requests: Option<u32>,
    max_requests_jitter: Option<u32>,
    warnings: Vec<String>,
}

pub fn launch_plan(options: &LaunchOptions) -> Result<LaunchPlan, WorkerLaunchError> {
    let backend_plan = backend_plan(options)?;
    if backend_plan.workers > 1 {
        return Err(WorkerLaunchError::UnsupportedRuntime);
    }
    let tls_enabled = options.certfile.is_some() && options.keyfile.is_some();
    Ok(LaunchPlan {
        backend: options.backend,
        bind: format!("{}:{}", options.host, options.port),
        workers: backend_plan.workers,
        timeout_seconds: backend_plan.timeout_seconds,
        tls: TlsPlan {
            enabled: tls_enabled,
            ciphers: backend_plan.ciphers,
        },
        process: ProcessPlan {
            preload: backend_plan.preload,
            child_exit_cleanup: backend_plan.child_exit_cleanup,
        },
        runtime_threads: backend_plan.runtime_threads,
        max_requests: backend_plan.max_requests,
        max_requests_jitter: backend_plan.max_requests_jitter,
        start_message: start_message(options, backend_plan.workers),
        warnings: backend_plan.warnings,
    })
}

fn backend_plan(options: &LaunchOptions) -> Result<BackendPlan, WorkerLaunchError> {
    match options.backend {
        Backend::Hypercorn => Ok(hypercorn_plan(options)),
        Backend::Granian => granian_plan(options),
        Backend::Gunicorn => Ok(gunicorn_plan(options)),
    }
}

fn hypercorn_plan(options: &LaunchOptions) -> BackendPlan {
    let tls_enabled = options.certfile.is_some() && options.keyfile.is_some();
    BackendPlan {
        workers: options.workers,
        timeout_seconds: None,
        ciphers: tls_enabled.then(|| options.ciphers.clone()).flatten(),
        preload: false,
        child_exit_cleanup: false,
        runtime_threads: None,
        max_requests: None,
        max_requests_jitter: None,
        warnings: Vec::new(),
    }
}

fn granian_plan(options: &LaunchOptions) -> Result<BackendPlan, WorkerLaunchError> {
    if options.certfile.is_some() != options.keyfile.is_some() {
        return Err(WorkerLaunchError::IncompleteGranianTls);
    }
    let mut warnings = Vec::new();
    if options.max_requests.is_some() {
        warnings.push("--max_requests_before_restart is not supported by Granian".to_owned());
    }
    if options.ciphers.is_some() {
        warnings.push("--ciphers is not applied when using --run_granian".to_owned());
    }
    Ok(BackendPlan {
        workers: options.workers.max(1),
        timeout_seconds: None,
        ciphers: None,
        preload: false,
        child_exit_cleanup: false,
        runtime_threads: options.runtime_threads,
        max_requests: None,
        max_requests_jitter: None,
        warnings,
    })
}

fn gunicorn_plan(options: &LaunchOptions) -> BackendPlan {
    let mut warnings = Vec::new();
    if options.max_requests_jitter.is_some() && options.max_requests.is_none() {
        warnings.push(
            "--max_requests_before_restart_jitter has no effect without --max_requests_before_restart"
                .to_owned(),
        );
    }
    BackendPlan {
        workers: options.workers,
        timeout_seconds: Some(600),
        ciphers: None,
        preload: true,
        child_exit_cleanup: options.prometheus_dir.is_some(),
        runtime_threads: None,
        max_requests: options.max_requests,
        max_requests_jitter: options
            .max_requests_jitter
            .filter(|_| options.max_requests.is_some()),
        warnings,
    }
}

fn start_message(options: &LaunchOptions, workers: u32) -> String {
    let metrics_suffix = options
        .metrics_port
        .map_or_else(String::new, |port| format!("; metrics port {port}"));
    let worker_suffix = match options.backend {
        Backend::Hypercorn => String::new(),
        Backend::Granian | Backend::Gunicorn => format!(" with {workers} workers"),
    };
    format!(
        "Starting server on {}:{}{} using {:?}{}",
        options.host, options.port, worker_suffix, options.backend, metrics_suffix
    )
}

#[must_use]
pub fn prometheus_callback_configured(
    settings: Option<&BTreeMap<String, CallbackSetting>>,
) -> bool {
    settings.is_some_and(|configured| {
        ["callbacks", "success_callback", "failure_callback"]
            .into_iter()
            .filter_map(|key| configured.get(key))
            .any(|setting| match setting {
                CallbackSetting::Single(name) => name == "prometheus",
                CallbackSetting::Multiple(names) => names.iter().any(|name| name == "prometheus"),
            })
    })
}

pub fn prometheus_directory(
    workers: u32,
    settings: Option<&BTreeMap<String, CallbackSetting>>,
    metrics_port: Option<u16>,
    environment: &BTreeMap<String, String>,
    mode: DirectoryMode,
) -> Result<Option<DirectoryPreparation>, DirectoryPreparationError> {
    if metrics_port.is_none() && (workers <= 1 || !prometheus_callback_configured(settings)) {
        return Ok(None);
    }
    let configured = environment
        .get("PROMETHEUS_MULTIPROC_DIR")
        .or_else(|| environment.get("prometheus_multiproc_dir"));
    let path = configured.map_or_else(
        || {
            let root = temp_root(environment);
            if root.as_os_str() == "." {
                PathBuf::from("litellm_prometheus_multiproc")
            } else {
                root.join("litellm_prometheus_multiproc")
            }
        },
        PathBuf::from,
    );
    let preparation = DirectoryPreparation {
        path,
        action: if configured.is_some() {
            "Using existing".to_owned()
        } else {
            "Auto-created".to_owned()
        },
        should_prepare: true,
    };
    if mode == DirectoryMode::Prepare {
        fs::create_dir_all(&preparation.path).map_err(|source| {
            DirectoryPreparationError::Create {
                path: preparation.path.clone(),
                source,
            }
        })?;
    }
    Ok(Some(preparation))
}

fn temp_root(environment: &BTreeMap<String, String>) -> PathBuf {
    let configured = TEMP_ROOT_ENV
        .into_iter()
        .filter_map(|key| environment.get(key))
        .find(|path| !path.is_empty());
    configured.map_or_else(|| PathBuf::from(DEFAULT_TEMP_ROOT), PathBuf::from)
}
