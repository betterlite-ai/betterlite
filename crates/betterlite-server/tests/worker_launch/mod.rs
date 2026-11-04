use betterlite_server::worker_options::callback_settings::CallbackSettings;
use betterlite_server::worker_options::*;
use rstest::rstest;
use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};

fn path_string(path: &Path) -> String {
    path.to_str().expect("test path is valid UTF-8").to_owned()
}

fn options(backend: Backend) -> LaunchOptions {
    LaunchOptions {
        backend,
        host: "127.0.0.1".to_owned(),
        port: 4000,
        workers: 1,
        runtime_threads: None,
        certfile: None,
        keyfile: None,
        ciphers: None,
        max_requests: None,
        max_requests_jitter: None,
        metrics_port: None,
        prometheus_dir: None,
    }
}

#[test]
fn hypercorn_launch_plan_has_bind_and_plain_asyncio_defaults() {
    let plan = launch_plan(&options(Backend::Hypercorn)).expect("launch plan");
    assert_eq!(plan.bind, "127.0.0.1:4000");
    assert!(!plan.tls.enabled);
    assert_eq!(plan.tls.ciphers, None);
    assert!(!plan.process.preload);
}

#[rstest]
#[case::hypercorn(Backend::Hypercorn)]
#[case::granian(Backend::Granian)]
#[case::gunicorn(Backend::Gunicorn)]
fn launch_plans_enable_tls_with_both_files(#[case] backend: Backend) {
    let mut launch = options(backend);
    launch.certfile = Some(PathBuf::from("cert.pem"));
    launch.keyfile = Some(PathBuf::from("key.pem"));
    launch.ciphers = Some("DEFAULT".to_owned());
    let plan = launch_plan(&launch).expect("launch plan");
    assert!(plan.tls.enabled);
    assert_eq!(
        plan.tls.ciphers.as_deref(),
        (backend == Backend::Hypercorn).then_some("DEFAULT")
    );
}

#[test]
fn granian_zero_workers_uses_one_worker() {
    let mut launch = options(Backend::Granian);
    launch.workers = 0;
    assert_eq!(launch_plan(&launch).expect("launch plan").workers, 1);
}

#[test]
fn granian_multiple_workers_are_rejected_by_native_runtime() {
    let mut launch = options(Backend::Granian);
    launch.workers = 2;
    assert_eq!(
        launch_plan(&launch),
        Err(WorkerLaunchError::UnsupportedRuntime)
    );
}

#[test]
fn granian_threads_and_unsupported_option_warnings_are_in_plan() {
    let mut launch = options(Backend::Granian);
    launch.runtime_threads = Some(4);
    launch.max_requests = Some(100);
    launch.ciphers = Some("HIGH".to_owned());
    let plan = launch_plan(&launch).expect("launch plan");
    assert_eq!(plan.runtime_threads, Some(4));
    assert_eq!(
        plan.warnings,
        [
            "--max_requests_before_restart is not supported by Granian (Granian uses workers_lifetime in seconds, not a per-request limit).",
            "--ciphers is not applied when using --run_granian.",
        ]
    );
}

#[test]
fn granian_rejects_incomplete_tls_pair() {
    let mut launch = options(Backend::Granian);
    launch.certfile = Some(PathBuf::from("cert.pem"));
    assert_eq!(
        launch_plan(&launch),
        Err(WorkerLaunchError::IncompleteGranianTls)
    );
}

#[test]
fn gunicorn_plan_has_preload_timeout_and_cleanup_only_with_metrics_dir() {
    let mut launch = options(Backend::Gunicorn);
    let plan = launch_plan(&launch).expect("launch plan");
    assert!(plan.process.preload);
    assert_eq!(plan.timeout_seconds, Some(600));
    assert!(!plan.process.child_exit_cleanup);
    launch.prometheus_dir = Some(PathBuf::from("/tmp/metrics"));
    assert!(
        launch_plan(&launch)
            .expect("launch plan")
            .process
            .child_exit_cleanup
    );
}

#[test]
fn gunicorn_request_limits_and_jitter_are_conditional() {
    let mut launch = options(Backend::Gunicorn);
    launch.max_requests = Some(80);
    launch.max_requests_jitter = Some(10);
    let plan = launch_plan(&launch).expect("launch plan");
    assert_eq!(plan.max_requests, Some(80));
    assert_eq!(plan.max_requests_jitter, Some(10));
    launch.max_requests = None;
    let jitter_only = launch_plan(&launch).expect("launch plan");
    assert_eq!(jitter_only.max_requests_jitter, None);
    assert_eq!(
        jitter_only.warnings,
        [
            "--max_requests_before_restart_jitter has no effect without --max_requests_before_restart"
        ]
    );
}

#[test]
fn start_message_and_metrics_message_are_separate() {
    let mut launch = options(Backend::Hypercorn);
    launch.metrics_port = Some(9000);
    let plan = launch_plan(&launch).expect("launch plan");
    assert_eq!(
        plan.start_message,
        "Starting server on 127.0.0.1:4000 using Hypercorn"
    );
    assert_eq!(
        plan.metrics_message.as_deref(),
        Some("Serving Prometheus metrics on 127.0.0.1:9000/metrics")
    );
    let mut granian = options(Backend::Granian);
    assert_eq!(
        launch_plan(&granian).expect("launch plan").start_message,
        "Starting server on 127.0.0.1:4000 using Granian"
    );
    granian.backend = Backend::Gunicorn;
    assert_eq!(
        launch_plan(&granian).expect("launch plan").start_message,
        "Starting server on 127.0.0.1:4000 with 1 workers"
    );
}

#[rstest]
#[case::string(r#"{"callbacks":"prometheus"}"#, true)]
#[case::list(r#"{"success_callback":["other",3,"prometheus"]}"#, true)]
#[case::failure_callback(r#"{"failure_callback":"prometheus"}"#, true)]
#[case::misspelled(r#"{"callbacks":["prometheu"]}"#, false)]
#[case::null(r#"{"callbacks":null}"#, false)]
fn callback_detection_checks_all_settings(#[case] input: &str, #[case] expected: bool) {
    let settings = serde_json::from_str::<CallbackSettings>(input).expect("settings map");
    assert_eq!(settings.is_configured(), expected);
}

#[test]
fn callback_detection_ignores_unrelated_settings() {
    let settings = serde_json::from_str::<CallbackSettings>(
        r#"{"drop_params":true,"callbacks":["prometheus"],"num_retries":3}"#,
    )
    .expect("settings map");
    assert!(settings.is_configured());
}

#[test]
fn metrics_listener_requests_shared_samples_without_callback_configuration() {
    let result = prometheus_directory(1, None, Some(9000), &BTreeMap::new(), DirectoryMode::Plan)
        .expect("directory plan");
    assert_eq!(
        result,
        Some(DirectoryPreparation {
            path: env::current_dir()
                .expect("current directory")
                .join("litellm_prometheus_multiproc"),
            action: "Auto-created".to_owned(),
            should_prepare: true,
        })
    );
}

#[test]
fn multiproc_directory_uses_lowercase_environment_variant() {
    let environment = BTreeMap::from([(
        "prometheus_multiproc_dir".to_owned(),
        "/var/tmp/prom".to_owned(),
    )]);
    let result = prometheus_directory(2, None, Some(9000), &environment, DirectoryMode::Plan)
        .expect("directory plan");
    assert_eq!(
        result.map(|value| value.path),
        Some(PathBuf::from("/var/tmp/prom"))
    );
}

#[rstest]
#[case::tmpdir("TMPDIR")]
#[case::temp("TEMP")]
#[case::tmp("TMP")]
fn multiproc_directory_checks_temp_root_environment(#[case] key: &str) {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let path = temporary.path().to_path_buf();
    let environment = BTreeMap::from([(key.to_owned(), path_string(&path))]);
    let result = prometheus_directory(1, None, Some(9000), &environment, DirectoryMode::Plan)
        .expect("directory plan");
    assert_eq!(
        result.map(|value| value.path),
        Some(path.join("litellm_prometheus_multiproc"))
    );
}

#[test]
fn multiproc_directory_prefers_tmpdir_then_temp_then_tmp() {
    let tmpdir = tempfile::tempdir().expect("TMPDIR");
    let temp = tempfile::tempdir().expect("TEMP");
    let third_root = tempfile::tempdir().expect("TMP");
    let environment = BTreeMap::from([
        ("TMPDIR".to_owned(), path_string(tmpdir.path())),
        ("TEMP".to_owned(), path_string(temp.path())),
        ("TMP".to_owned(), path_string(third_root.path())),
    ]);
    let result = prometheus_directory(1, None, Some(9000), &environment, DirectoryMode::Plan)
        .expect("directory plan");
    assert_eq!(
        result.map(|value| value.path),
        Some(tmpdir.path().join("litellm_prometheus_multiproc"))
    );
}

#[test]
fn multiproc_directory_skips_missing_tmpdir_candidate() {
    let temp = tempfile::tempdir().expect("TEMP");
    let environment = BTreeMap::from([
        (
            "TMPDIR".to_owned(),
            path_string(&temp.path().join("missing")),
        ),
        ("TEMP".to_owned(), path_string(temp.path())),
    ]);
    let result = prometheus_directory(1, None, Some(9000), &environment, DirectoryMode::Plan)
        .expect("directory plan");
    assert_eq!(
        result.map(|value| value.path),
        Some(temp.path().join("litellm_prometheus_multiproc"))
    );
}

#[test]
fn empty_uppercase_multiproc_directory_uses_temp_root() {
    let temp = tempfile::tempdir().expect("temporary root");
    let environment = BTreeMap::from([
        ("PROMETHEUS_MULTIPROC_DIR".to_owned(), String::new()),
        ("TMPDIR".to_owned(), path_string(temp.path())),
    ]);
    let result = prometheus_directory(2, None, Some(9000), &environment, DirectoryMode::Plan)
        .expect("directory plan")
        .expect("directory decision");
    assert_eq!(
        result.path,
        temp.path().join("litellm_prometheus_multiproc")
    );
    assert_eq!(result.action, "Auto-created");
}

#[test]
fn empty_uppercase_multiproc_directory_falls_through_to_lowercase_value() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let environment = BTreeMap::from([
        ("PROMETHEUS_MULTIPROC_DIR".to_owned(), String::new()),
        (
            "prometheus_multiproc_dir".to_owned(),
            path_string(temporary.path()),
        ),
    ]);
    let result = prometheus_directory(2, None, Some(9000), &environment, DirectoryMode::Plan)
        .expect("directory plan")
        .expect("directory decision");
    assert_eq!(result.path, temporary.path());
    assert_eq!(result.action, "Using existing");
}

#[test]
fn multiproc_directory_is_created_when_prepared() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let metrics_path = temporary.path().join("metrics");
    let environment = BTreeMap::from([(
        "PROMETHEUS_MULTIPROC_DIR".to_owned(),
        metrics_path
            .to_str()
            .expect("temporary path is valid UTF-8")
            .to_owned(),
    )]);
    prometheus_directory(2, None, Some(9000), &environment, DirectoryMode::Prepare)
        .expect("directory preparation");
    assert!(metrics_path.is_dir());
}
