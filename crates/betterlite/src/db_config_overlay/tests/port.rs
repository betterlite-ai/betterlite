use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::table_ops::rows::RowError;
use betterlite_server::config_overlay_port::{ConfigOverlayPort, ConfigOverlayPortError};
use tempfile::TempDir;
use tokio::sync::Mutex;

use super::support::{
    RecordingInstaller, RecordingRuntime, database, failing_database, file_source, json, state,
    write_config,
};
use crate::db_config_overlay::port::OverlayPortAdapter;
use crate::db_config_overlay::reload::reload_config;
use crate::db_config_overlay::state::OverlayDatabase;

const CONFIG: &str =
    "model_list:\n  - model_name: gpt\ngeneral_settings:\n  master_key: os.environ/MASTER\n";

// the port and the recording installer behind it, so a test reads what was installed
struct Port {
    port: Arc<dyn ConfigOverlayPort>,
    installer: Arc<RecordingInstaller>,
}

fn port(path: &Path, database: Option<OverlayDatabase>, recording: RecordingInstaller) -> Port {
    let installer = Arc::new(recording);
    let environment = BTreeMap::from([("MASTER".to_owned(), "sk-env".to_owned())]);
    let adapter = OverlayPortAdapter {
        state: Arc::new(Mutex::new(state(
            true,
            &Arc::new(RecordingRuntime::default()),
        ))),
        source: file_source(path),
        database,
        environment: Arc::new(Mutex::new(environment)),
        installer: Arc::<RecordingInstaller>::clone(&installer),
    };
    Port {
        port: Arc::new(adapter),
        installer,
    }
}

#[tokio::test]
async fn port_reload_returns_what_reload_config_returns() {
    let directory = TempDir::new().unwrap();
    let path = write_config(directory.path(), CONFIG);
    let rows = || database(&[("router_settings", json!({"num_retries": 2}))]);
    let through_port = port(&path, Some(rows()), RecordingInstaller::default())
        .port
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
        .port
        .reload_config()
        .await
        .unwrap_err();
    assert_eq!(error, ConfigOverlayPortError::Reload);
}

#[tokio::test]
async fn port_serves_the_endpoints_in_order() {
    let directory = TempDir::new().unwrap();
    let path = write_config(directory.path(), CONFIG);
    let served = port(&path, None, RecordingInstaller::default());
    let endpoints = [json!({"path": "/b"}), json!({"path": "/a"})];
    served
        .port
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
        .port
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
        fails: true,
        ..RecordingInstaller::default()
    };
    let served = port(&path, None, failing);
    let error = served
        .port
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
    .port
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
    .port
    .reload_config()
    .await
    .unwrap_err();
    assert_eq!(error, ConfigOverlayPortError::Reload);
}
