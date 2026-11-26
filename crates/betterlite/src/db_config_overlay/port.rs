use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::{includes::IncludeError, settings::JsonValue};
use betterlite_db::database_faults::native::is_native_transport_error;
use betterlite_server::bucket_config::error::BucketConfigError;
use betterlite_server::config_overlay_port::ConfigOverlayPortError::{PassThrough, Reload};
use betterlite_server::config_overlay_port::{ConfigOverlayPort, ConfigOverlayPortError};
use betterlite_server::proxy_config_state::snapshot::ConfigWithBaseline;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_util::task::TaskTracker;

use super::environ::Workers;
use super::error::OverlayError;
use super::pass_through::{
    PassThroughInstaller, install_pass_through_endpoints, publish_pass_through,
    validate_db_endpoints,
};
use super::reload::{
    apply_stored_rows, finish_reload, prepare_reload, reload_stores, resolve_reload,
};
use super::rows::fetch_db_rows;
use super::source::{ConfigSource, read_config};
use super::state::{OverlayDatabase, OverlayState};

pub struct OverlayParts {
    pub state: Arc<Mutex<OverlayState>>,
    pub source: ConfigSource,
    pub database: Option<OverlayDatabase>,
    pub environment: Arc<Mutex<BTreeMap<String, String>>>,
    pub installer: Arc<dyn PassThroughInstaller>,
    pub workers: Arc<Workers>,
}

pub enum Mutation {
    Reload(oneshot::Sender<Result<ConfigWithBaseline, OverlayError>>),
    Serve(Vec<JsonValue>, oneshot::Sender<Result<(), OverlayError>>),
}

// LiteLLM's event loop runs a reload and a serve one after the other: one owned worker drains this queue, so no guard spans a read, a fetch, a lookup or an install.
pub struct OverlayPortAdapter {
    pub mutations: mpsc::Sender<Mutation>,
    pub tasks: TaskTracker,
}

impl OverlayPortAdapter {
    #[must_use]
    pub fn start(ctx: OverlayParts) -> Self {
        let (mutations, mut queue) = mpsc::channel(1);
        let tasks = TaskTracker::new();
        let _worker = tasks.spawn(async move {
            while let Some(mutation) = queue.recv().await {
                match mutation {
                    Mutation::Reload(reply) => drop(reply.send(run_reload(&ctx).await)),
                    Mutation::Serve(list, reply) => drop(reply.send(run_serve(&ctx, &list).await)),
                }
            }
        });
        Self { mutations, tasks }
    }
}

#[async_trait]
impl ConfigOverlayPort for OverlayPortAdapter {
    async fn reload_config(&self) -> Result<ConfigWithBaseline, ConfigOverlayPortError> {
        let (reply, answer) = oneshot::channel();
        let queued = self.mutations.send(Mutation::Reload(reply)).await;
        queued.map_err(|_closed| Reload)?;
        let outcome = answer.await.unwrap_or(Err(OverlayError::WorkerUnavailable));
        outcome.map_err(|error| port_error(&error, Reload))
    }

    async fn serve_pass_through_endpoints(
        &self,
        db_endpoints: &[JsonValue],
    ) -> Result<(), ConfigOverlayPortError> {
        let (reply, answer) = oneshot::channel();
        let mutation = Mutation::Serve(db_endpoints.to_vec(), reply);
        let queued = self.mutations.send(mutation).await;
        queued.map_err(|_closed| PassThrough)?;
        let outcome = answer.await.unwrap_or(Err(OverlayError::WorkerUnavailable));
        outcome.map_err(|error| port_error(&error, PassThrough))
    }
}

// get_config's turn, with every state guard confined to the synchronous step it protects
async fn run_reload(parts: &OverlayParts) -> Result<ConfigWithBaseline, OverlayError> {
    let initial = parts.environment.lock().await.clone();
    let file_config = read_config(&parts.source, &initial, &parts.workers).await?;
    let mut locked = parts.state.lock().await;
    reload_stores(&mut locked, &file_config)?;
    let store_model_in_db = locked.config.store_model_in_db;
    drop(locked);
    let stored = match parts.database.as_ref().filter(|_| store_model_in_db) {
        Some(backed) => Some((backed, fetch_db_rows(backed).await?)),
        None => None,
    };
    let mut shared = parts.environment.lock().await.clone();
    let mut state = parts.state.lock().await;
    let written = apply_stored_rows(&mut state, stored, &mut shared);
    let pending = prepare_reload(&mut state, &file_config, &parts.source, &shared);
    drop(state);
    let merged = {
        let mut current = parts.environment.lock().await;
        current.extend(written);
        current.clone()
    };
    let resolved = resolve_reload(pending?, merged, &parts.workers).await?;
    let mut settled = parts.state.lock().await;
    Ok(finish_reload(&mut settled, resolved))
}

// _serve_pass_through_endpoints, with the state guard gone before the installer runs
async fn run_serve(parts: &OverlayParts, endpoints: &[JsonValue]) -> Result<(), OverlayError> {
    let mut state = parts.state.lock().await;
    publish_pass_through(&mut state, endpoints)?;
    drop(state);
    validate_db_endpoints(endpoints)?;
    install_pass_through_endpoints(endpoints, parts.installer.as_ref()).await
}

// The log keeps the typed cause but never the YAML source excerpt: a parser renders the neighboring lines, and a config file carries keys.
fn port_error(error: &OverlayError, step: ConfigOverlayPortError) -> ConfigOverlayPortError {
    let message = if let OverlayError::ConfigYaml { source, .. } = error {
        format!("{error}: {}", source.without_snippet())
    } else if let OverlayError::Includes(IncludeError::Yaml { path, source })
    | OverlayError::Bucket(BucketConfigError::Include(IncludeError::Yaml {
        path,
        source,
    })) = error
    {
        let file = path.display();
        let reason = source.without_snippet();
        format!("invalid YAML in config include {file}: {reason}")
    } else {
        error.to_string()
    };
    tracing::error!(error = %message, "the config overlay failed");
    if let OverlayError::Database { source } = error
        && is_native_transport_error(source)
    {
        return ConfigOverlayPortError::DatabaseUnavailable;
    }
    step
}
