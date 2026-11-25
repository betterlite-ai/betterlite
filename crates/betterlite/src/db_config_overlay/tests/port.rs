use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::table_ops::rows::RowError;
use betterlite_secret_managers::secret_lookup::ports::OidcHttp;
use betterlite_server::config_overlay_port::{ConfigOverlayPort, ConfigOverlayPortError};
use tempfile::TempDir;
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::Semaphore;
use tokio::task::{JoinSet, yield_now};

use super::support::{
    Journal, OnceOidc, RecordingInstaller, RecordingRuntime, blocking_oidc, database,
    failing_database, file_source, gated_database, json, reload_config, state, write_config,
};
use crate::config_boot::test_support::capture_logs;
use crate::db_config_overlay::environ::{WORKER_LIMIT, Workers};
use crate::db_config_overlay::port::{OverlayParts, OverlayPortAdapter};
use crate::db_config_overlay::state::{OverlayDatabase, OverlayState, PASS_THROUGH_ENDPOINTS};

const CONFIG: &str =
    "model_list:\n  - model_name: gpt\ngeneral_settings:\n  master_key: os.environ/MASTER\n";
const OIDC_CONFIG: &str = "general_settings:\n  master_key: os.environ/oidc/google/audience\n";

// the port, the recording installer behind it, and the state and environment it mutates, so a test reads what the reload committed
struct Port {
    overlay: Arc<dyn ConfigOverlayPort>,
    installer: Arc<RecordingInstaller>,
    state: Arc<AsyncMutex<OverlayState>>,
    environment: Arc<AsyncMutex<BTreeMap<String, String>>>,
}

fn port(path: &Path, database: Option<OverlayDatabase>, recording: RecordingInstaller) -> Port {
    port_with_oidc(path, database, recording, None)
}

fn port_with_oidc(
    path: &Path,
    database: Option<OverlayDatabase>,
    recording: RecordingInstaller,
    oidc: Option<Arc<dyn OidcHttp>>,
) -> Port {
    let installer = Arc::new(recording);
    let environment = Arc::new(AsyncMutex::new(BTreeMap::from([(
        "MASTER".to_owned(),
        "sk-env".to_owned(),
    )])));
    let mut initial_state = state(true, &Arc::new(RecordingRuntime::default()));
    if let Some(endpoint) = oidc {
        initial_state.secrets.lookup.oidc_http = endpoint;
    }
    let state = Arc::new(AsyncMutex::new(initial_state));
    let parts = OverlayParts {
        state: Arc::clone(&state),
        source: file_source(path),
        database,
        environment: Arc::clone(&environment),
        installer: Arc::<RecordingInstaller>::clone(&installer),
        workers: Arc::new(Workers {
            slots: Arc::new(Semaphore::new(WORKER_LIMIT)),
            live: Mutex::new(Vec::new()),
        }),
    };
    Port {
        overlay: Arc::new(OverlayPortAdapter::start(parts)),
        installer,
        state,
        environment,
    }
}

#[tokio::test]
async fn port_reload_returns_what_reload_config_returns() {
    let directory = TempDir::new().unwrap();
    let path = write_config(directory.path(), CONFIG);
    let rows = || database(&[("router_settings", json!({"num_retries": 2}))]);
    let through_port = port(&path, Some(rows()), RecordingInstaller::default())
        .overlay
        .reload_config()
        .await
        .unwrap();
    let mut direct_state = state(true, &Arc::new(RecordingRuntime::default()));
    let mut environment = BTreeMap::from([("MASTER".to_owned(), "sk-env".to_owned())]);
    let direct = reload_config(
        &mut direct_state,
        &file_source(&path),
        Some(&rows()),
        &mut environment,
    )
    .await
    .unwrap();
    assert_eq!(through_port, direct);
    assert_eq!(
        through_port.values["general_settings"],
        json!({"master_key": "sk-env"})
    );
    assert_eq!(
        through_port.values["router_settings"],
        json!({"num_retries": 2})
    );
}

#[tokio::test]
async fn port_missing_config_file_gives_reload() {
    let directory = TempDir::new().unwrap();
    let missing = directory.path().join("absent.yaml");
    let error = port(&missing, None, RecordingInstaller::default())
        .overlay
        .reload_config()
        .await
        .unwrap_err();
    assert_eq!(error, ConfigOverlayPortError::Reload);
}

// A parser diagnostic renders the surrounding config lines, so the logged error must not carry the file text
#[tokio::test]
async fn port_reload_logs_the_yaml_error_without_the_config_text() {
    let (logs, _guard) = capture_logs();
    let directory = TempDir::new().unwrap();
    let path = write_config(
        directory.path(),
        "general_settings: {master_key: sk-live-secret, ]}\n",
    );
    let error = port(&path, None, RecordingInstaller::default())
        .overlay
        .reload_config()
        .await
        .unwrap_err();
    assert_eq!(error, ConfigOverlayPortError::Reload);
    let logged = logs.contents();
    assert!(logged.contains("is not valid YAML"), "{logged}");
    assert!(!logged.contains("sk-live-secret"), "{logged}");
    assert!(!logged.contains("general_settings"), "{logged}");
}

#[tokio::test]
async fn port_serves_the_endpoints_in_order() {
    let directory = TempDir::new().unwrap();
    let path = write_config(directory.path(), CONFIG);
    let served = port(&path, None, RecordingInstaller::default());
    let endpoints = [json!({"path": "/b"}), json!({"path": "/a"})];
    served
        .overlay
        .serve_pass_through_endpoints(&endpoints)
        .await
        .unwrap();
    assert_eq!(served.installer.calls(), [endpoints.to_vec()]);
}

#[tokio::test]
async fn port_non_object_entry_gives_pass_through_and_installs_nothing() {
    let directory = TempDir::new().unwrap();
    let path = write_config(directory.path(), CONFIG);
    let served = port(&path, None, RecordingInstaller::default());
    let endpoints = [json!({"path": "/a"}), json!("/b")];
    let error = served
        .overlay
        .serve_pass_through_endpoints(&endpoints)
        .await
        .unwrap_err();
    assert_eq!(error, ConfigOverlayPortError::PassThrough);
    assert!(served.installer.calls().is_empty());
}

#[tokio::test]
async fn port_install_failure_gives_pass_through() {
    let directory = TempDir::new().unwrap();
    let path = write_config(directory.path(), CONFIG);
    let failing = RecordingInstaller {
        journal: Journal::default(),
        fails: true,
        calls: Mutex::new(Vec::new()),
    };
    let served = port(&path, None, failing);
    let error = served
        .overlay
        .serve_pass_through_endpoints(&[json!({"path": "/a"})])
        .await
        .unwrap_err();
    assert_eq!(error, ConfigOverlayPortError::PassThrough);
}

#[tokio::test]
async fn port_database_unavailable_on_no_db_connected() {
    let directory = TempDir::new().unwrap();
    let path = write_config(directory.path(), CONFIG);
    let failure: fn() -> DbError<RowError> = || DbError::NoDbConnected;
    let error = port(
        &path,
        Some(failing_database(failure)),
        RecordingInstaller::default(),
    )
    .overlay
    .reload_config()
    .await
    .unwrap_err();
    assert_eq!(error, ConfigOverlayPortError::DatabaseUnavailable);
}

#[tokio::test]
async fn port_database_unavailable_not_for_create_null() {
    let directory = TempDir::new().unwrap();
    let path = write_config(directory.path(), CONFIG);
    let failure: fn() -> DbError<RowError> = || DbError::CreateNull;
    let error = port(
        &path,
        Some(failing_database(failure)),
        RecordingInstaller::default(),
    )
    .overlay
    .reload_config()
    .await
    .unwrap_err();
    assert_eq!(error, ConfigOverlayPortError::Reload);
}

// The token an earlier reload fetched is in the lookup the next reload copies, so its resolution needs no endpoint.
#[tokio::test]
async fn port_reuses_the_oidc_token_an_earlier_reload_fetched() {
    let directory = TempDir::new().unwrap();
    let path = write_config(directory.path(), OIDC_CONFIG);
    let oidc = Arc::new(OnceOidc {
        calls: AtomicUsize::new(0),
    });
    let served = port_with_oidc(
        &path,
        None,
        RecordingInstaller::default(),
        Some(Arc::<OnceOidc>::clone(&oidc)),
    );
    let first = served.overlay.reload_config().await.unwrap();
    let second = served.overlay.reload_config().await.unwrap();
    let master_key = json!({"master_key": "google-token"});
    assert_eq!(first.values["general_settings"], master_key);
    assert_eq!(second.values["general_settings"], master_key);
    assert_eq!(oidc.calls.load(Ordering::Relaxed), 1);
}

// A serve that arrives while a reload resolves must survive that reload's older pending config.
#[tokio::test]
async fn port_keeps_endpoints_served_while_a_reload_resolves() {
    let directory = TempDir::new().unwrap();
    let path = write_config(directory.path(), OIDC_CONFIG);
    let (oidc, started, release) = blocking_oidc();
    let served = port_with_oidc(&path, None, RecordingInstaller::default(), Some(oidc));
    let mut tasks = JoinSet::new();
    tasks.spawn({
        let port = Arc::clone(&served.overlay);
        async move { port.reload_config().await.map(|_config| ()) }
    });
    started.await.unwrap();
    tasks.spawn({
        let port = Arc::clone(&served.overlay);
        async move {
            port.serve_pass_through_endpoints(&[json!({"path": "/served"})])
                .await
        }
    });
    yield_now().await;
    release.send(()).unwrap();
    tasks.join_next().await.unwrap().unwrap().unwrap();
    tasks.join_next().await.unwrap().unwrap().unwrap();
    let expected = json!([{"path": "/served"}]);
    let state = served.state.lock().await;
    assert_eq!(
        state.config.general_settings.get(PASS_THROUGH_ENDPOINTS),
        Some(&expected)
    );
}

// A database read is pending when another task rewrites the environment: the reload resolves against the new values, and only the database's own keys join the shared map.
#[tokio::test]
async fn port_keeps_a_concurrent_environment_update_during_a_database_read() {
    let directory = TempDir::new().unwrap();
    let path = write_config(
        directory.path(),
        "general_settings:\n  master_key: os.environ/SHARED\n",
    );
    let (database, mut started, gate) =
        gated_database(&[("environment_variables", json!({"db_secret": "db-value"}))]);
    let served = port(&path, Some(database), RecordingInstaller::default());
    {
        let mut environment = served.environment.lock().await;
        environment.insert("SHARED".to_owned(), "before".to_owned());
        environment.insert("STALE".to_owned(), "kept".to_owned());
    }
    let mut tasks = JoinSet::new();
    tasks.spawn({
        let port = Arc::clone(&served.overlay);
        async move { port.reload_config().await }
    });
    started.recv().await.unwrap();
    {
        let mut environment = served.environment.lock().await;
        environment.insert("SHARED".to_owned(), "after".to_owned());
        environment.remove("STALE");
        environment.insert("CONCURRENT".to_owned(), "kept".to_owned());
    }
    gate.add_permits(1);
    let reloaded = tasks.join_next().await.unwrap().unwrap().unwrap();
    assert_eq!(
        reloaded.values["general_settings"],
        json!({"master_key": "after"})
    );
    let environment = served.environment.lock().await;
    assert_eq!(environment.get("SHARED").map(String::as_str), Some("after"));
    assert!(!environment.contains_key("STALE"));
    assert_eq!(
        environment.get("CONCURRENT").map(String::as_str),
        Some("kept")
    );
    assert_eq!(
        environment.get("DB_SECRET").map(String::as_str),
        Some("db-value")
    );
    assert_eq!(
        environment.get("db_secret").map(String::as_str),
        Some("db-value")
    );
}
