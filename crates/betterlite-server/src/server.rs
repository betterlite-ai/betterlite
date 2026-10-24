use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::Path;

use axum::serve;
use betterlite_config::loader::{ConfigError, ConfigLoader};
use thiserror::Error;
use tokio::net::TcpListener;
#[cfg(unix)]
use tokio::signal::unix::{Signal, SignalKind, signal};
use tokio::time::{Duration, timeout};

use crate::routes::router;

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("failed to read configuration {path}: {source}")]
    ReadConfig {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("invalid configuration: {0}")]
    Config(#[from] ConfigError),
    #[error("failed to bind {address}: {source}")]
    Bind {
        address: String,
        #[source]
        source: io::Error,
    },
    #[error("server failed: {0}")]
    Serve(#[source] io::Error),
}

pub async fn run(config_path: &Path, host: &str, port: u16) -> Result<(), ServerError> {
    let config_text =
        fs::read_to_string(config_path).map_err(|source| ServerError::ReadConfig {
            path: config_path.display().to_string(),
            source,
        })?;
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&config_text)?;
    let address = format!("{host}:{port}");
    let socket_address = address
        .parse::<SocketAddr>()
        .map_err(|source| ServerError::Bind {
            address: address.clone(),
            source: io::Error::new(io::ErrorKind::InvalidInput, source),
        })?;
    let listener = TcpListener::bind(socket_address)
        .await
        .map_err(|source| ServerError::Bind { address, source })?;
    tracing::info!(address = ?socket_address, "betterlite server listening");
    serve(listener, router(config.general_settings.master_key))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(ServerError::Serve)
}

#[cfg(unix)]
async fn shutdown_signal() {
    let mut terminate = match signal(SignalKind::terminate()) {
        Ok(terminate) => terminate,
        Err(error) => {
            tracing::error!(error = ?error, "failed to register SIGTERM handler");
            return;
        }
    };
    let mut interrupt = match signal(SignalKind::interrupt()) {
        Ok(interrupt) => interrupt,
        Err(error) => {
            tracing::error!(error = ?error, "failed to register Ctrl-C handler");
            return;
        }
    };
    loop {
        if signal_received(&mut terminate).await || signal_received(&mut interrupt).await {
            break;
        }
    }
}

#[cfg(unix)]
async fn signal_received(signal: &mut Signal) -> bool {
    match timeout(Duration::from_millis(100), signal.recv()).await {
        Ok(Some(()) | None) => true,
        Err(_) => false,
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(error = ?error, "failed to receive Ctrl-C");
    }
}
