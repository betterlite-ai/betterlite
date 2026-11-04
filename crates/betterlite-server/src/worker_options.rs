use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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

pub fn launch_plan(options: &LaunchOptions) -> Result<LaunchPlan, WorkerLaunchError> {
    validate_tls(options)?;
    if options.workers > 1 {
        return Err(WorkerLaunchError::UnsupportedRuntime);
    }
    let workers = if options.backend == Backend::Granian {
        options.workers.max(1)
    } else {
        options.workers
    };
    let tls_enabled = options.certfile.is_some() && options.keyfile.is_some();
    Ok(LaunchPlan {
        backend: options.backend.clone(),
        bind: format!("{}:{}", options.host, options.port),
        workers,
        timeout_seconds: (options.backend == Backend::Gunicorn).then_some(600),
        tls: TlsPlan {
            enabled: tls_enabled,
            ciphers: (options.backend == Backend::Hypercorn && tls_enabled)
                .then(|| options.ciphers.clone())
                .flatten(),
        },
        process: ProcessPlan {
            preload: options.backend == Backend::Gunicorn,
            child_exit_cleanup: options.backend == Backend::Gunicorn
                && options.prometheus_dir.is_some(),
        },
        runtime_threads: (options.backend == Backend::Granian)
            .then_some(options.runtime_threads)
            .flatten(),
        max_requests: (options.backend == Backend::Gunicorn)
            .then_some(options.max_requests)
            .flatten(),
        max_requests_jitter: (options.backend == Backend::Gunicorn
            && options.max_requests.is_some())
        .then_some(options.max_requests_jitter)
        .flatten(),
        start_message: start_message(options, workers),
        warnings: launch_warnings(options),
    })
}

fn validate_tls(options: &LaunchOptions) -> Result<(), WorkerLaunchError> {
    if options.backend == Backend::Granian
        && options.certfile.is_some() != options.keyfile.is_some()
    {
        return Err(WorkerLaunchError::IncompleteGranianTls);
    }
    Ok(())
}

fn launch_warnings(options: &LaunchOptions) -> Vec<String> {
    let mut warnings = Vec::new();
    if options.backend == Backend::Granian {
        if options.max_requests.is_some() {
            warnings.push("--max_requests_before_restart is not supported by Granian".to_owned());
        }
        if options.ciphers.is_some() {
            warnings.push("--ciphers is not applied when using --run_granian".to_owned());
        }
    }
    if options.backend == Backend::Gunicorn
        && options.max_requests_jitter.is_some()
        && options.max_requests.is_none()
    {
        warnings.push(
            "--max_requests_before_restart_jitter has no effect without --max_requests_before_restart"
                .to_owned(),
        );
    }
    warnings
}

fn start_message(options: &LaunchOptions, workers: u32) -> String {
    let metrics_suffix = options
        .metrics_port
        .map_or_else(String::new, |port| format!("; metrics port {port}"));
    let worker_suffix = if options.backend == Backend::Hypercorn {
        String::new()
    } else {
        format!(" with {workers} workers")
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
        || env::temp_dir().join("litellm_prometheus_multiproc"),
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
