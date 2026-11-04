use betterlite_server::worker_options::*;
use rstest::rstest;
use std::collections::BTreeMap;
use std::env;
use std::path::PathBuf;

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

#[rstest]
#[case::string("callbacks", CallbackSetting::Single("prometheus".to_owned()), true)]
#[case::list("success_callback", CallbackSetting::Multiple(vec!["other".to_owned(), "prometheus".to_owned()]), true)]
#[case::failure_callback("failure_callback", CallbackSetting::Single("prometheus".to_owned()), true)]
#[case::misspelled("callbacks", CallbackSetting::Multiple(vec!["prometheu".to_owned()]), false)]
fn callback_detection_checks_all_settings(
    #[case] key: &str,
    #[case] value: CallbackSetting,
    #[case] expected: bool,
) {
    let settings = BTreeMap::from([(key.to_owned(), value)]);
    assert_eq!(prometheus_callback_configured(Some(&settings)), expected);
}

#[test]
fn metrics_listener_requests_shared_samples_without_callback_configuration() {
    let result = prometheus_directory(1, None, Some(9000), &BTreeMap::new(), DirectoryMode::Plan)
        .expect("directory plan");
    assert_eq!(
        result,
        Some(DirectoryPreparation {
            path: env::temp_dir().join("litellm_prometheus_multiproc"),
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
