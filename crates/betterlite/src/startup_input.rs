use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream};
use std::path::Path;
use std::time::Duration;

use rand::Rng;
use rand::RngExt as _;
use serde::Deserialize;

const MAX_INPUT_BYTES: usize = 0x0001_0000;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const DEFAULT_PORT: u16 = 4_000;

pub const FALLBACK_PORT_LOW: u16 = 1_024;
pub const FALLBACK_PORT_HIGH: u16 = 49_152;

#[derive(Debug, thiserror::Error)]
pub enum StartupInputError {
    #[error("Unknown command: {command}")]
    UnknownCommand { command: String },
    #[error("the --headers value is longer than {limit} bytes")]
    HeadersTooLong { limit: usize },
    #[error("the --headers value is not a JSON object of strings: {source}")]
    Headers {
        #[source]
        source: serde_json::Error,
    },
    #[error("WORKER_CONFIG is longer than {limit} bytes")]
    WorkerConfigTooLong { limit: usize },
    #[error("WORKER_CONFIG is not a JSON object of boolean flags: {source}")]
    WorkerConfig {
        #[source]
        source: serde_json::Error,
    },
    #[error("--prometheus_metrics_port must differ from --port")]
    MetricsPortClash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionalCommand {
    None,
    XaiOauthLogin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
pub struct WorkerDebugFlags {
    pub debug: Option<bool>,
    pub detailed_debug: Option<bool>,
}

pub fn classify_positional(args: &[String]) -> Result<PositionalCommand, StartupInputError> {
    match args {
        [] => Ok(PositionalCommand::None),
        [first, second] if first == "xai-oauth" && second == "login" => {
            Ok(PositionalCommand::XaiOauthLogin)
        }
        _ => Err(StartupInputError::UnknownCommand {
            command: args.join(" "),
        }),
    }
}

#[must_use]
pub fn xai_login_lines(auth_file: &Path, expires_at: Option<i64>) -> Vec<String> {
    let mut lines = vec![format!(
        "xAI OAuth login successful. Credentials saved to {}.",
        auth_file.display()
    )];
    if let Some(expiry) = expires_at {
        lines.push(format!("Access token expires at {expiry}."));
    }
    lines
}

pub fn decode_headers(raw: &str) -> Result<Option<BTreeMap<String, String>>, StartupInputError> {
    if raw.is_empty() {
        return Ok(None);
    }
    if raw.len() > MAX_INPUT_BYTES {
        return Err(StartupInputError::HeadersTooLong {
            limit: MAX_INPUT_BYTES,
        });
    }
    let headers: BTreeMap<String, String> =
        serde_json::from_str(raw).map_err(|source| StartupInputError::Headers { source })?;
    Ok(Some(headers))
}

pub fn decode_worker_config(raw: &str) -> Result<WorkerDebugFlags, StartupInputError> {
    if raw.len() > MAX_INPUT_BYTES {
        return Err(StartupInputError::WorkerConfigTooLong {
            limit: MAX_INPUT_BYTES,
        });
    }
    serde_json::from_str(raw).map_err(|source| StartupInputError::WorkerConfig { source })
}

pub trait PortProber {
    fn in_use(&self, port: u16) -> bool;
}

pub struct LocalPortProber;

impl PortProber for LocalPortProber {
    fn in_use(&self, port: u16) -> bool {
        // LiteLLM's AF_INET socket resolves localhost over IPv4 only, so ::1 is never probed
        let address = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port));
        TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).is_ok()
    }
}

pub fn choose_port<R>(port: u16, prober: &dyn PortProber, rng: &mut R) -> u16
where
    R: Rng + ?Sized,
{
    if port == DEFAULT_PORT && prober.in_use(port) {
        rng.random_range(FALLBACK_PORT_LOW..=FALLBACK_PORT_HIGH)
    } else {
        port
    }
}

pub fn check_metrics_port(metrics: Option<u16>, port: u16) -> Result<(), StartupInputError> {
    if metrics == Some(port) {
        return Err(StartupInputError::MetricsPortClash);
    }
    Ok(())
}

mod tests;
