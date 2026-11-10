use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream};
use std::path::Path;
use std::time::Duration;

use rand::Rng;
use rand::RngExt as _;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::error::Category;

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
    #[error("the --headers value is not valid JSON ({category} at line {line}, column {column})")]
    HeadersSyntax {
        category: &'static str,
        line: usize,
        column: usize,
    },
    #[error("the --headers value must be a JSON object whose values are strings")]
    HeadersShape,
    #[error("WORKER_CONFIG is longer than {limit} bytes")]
    WorkerConfigTooLong { limit: usize },
    #[error("WORKER_CONFIG is not valid JSON ({category} at line {line}, column {column})")]
    WorkerConfigSyntax {
        category: &'static str,
        line: usize,
        column: usize,
    },
    #[error("WORKER_CONFIG must be a JSON object")]
    WorkerConfigShape,
    #[error("WORKER_CONFIG key {key} must be true or false")]
    WorkerConfigFlag { key: &'static str },
    #[error("--prometheus_metrics_port must differ from --port")]
    MetricsPortClash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionalCommand {
    None,
    XaiOauthLogin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
    match serde_json::from_str(raw) {
        Ok(headers) => Ok(Some(headers)),
        Err(error) if error.classify() == Category::Data => Err(StartupInputError::HeadersShape),
        Err(error) => Err(StartupInputError::HeadersSyntax {
            category: syntax_category(&error),
            line: error.line(),
            column: error.column(),
        }),
    }
}

fn syntax_category(error: &serde_json::Error) -> &'static str {
    if error.classify() == Category::Eof {
        "unexpected end of input"
    } else {
        "syntax error"
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Absent(()),
    Set(bool),
    NotBoolean(IgnoredAny),
}

fn read_flag(
    object: &BTreeMap<String, RawFlag>,
    key: &'static str,
) -> Result<Option<bool>, StartupInputError> {
    match object.get(key) {
        None | Some(RawFlag::Absent(())) => Ok(None),
        Some(RawFlag::Set(flag)) => Ok(Some(*flag)),
        Some(RawFlag::NotBoolean(_)) => Err(StartupInputError::WorkerConfigFlag { key }),
    }
}

pub fn decode_worker_config(raw: &str) -> Result<WorkerDebugFlags, StartupInputError> {
    if raw.len() > MAX_INPUT_BYTES {
        return Err(StartupInputError::WorkerConfigTooLong {
            limit: MAX_INPUT_BYTES,
        });
    }
    let object: BTreeMap<String, RawFlag> = match serde_json::from_str(raw) {
        Ok(object) => object,
        Err(error) if error.classify() == Category::Data => {
            return Err(StartupInputError::WorkerConfigShape);
        }
        Err(error) => {
            return Err(StartupInputError::WorkerConfigSyntax {
                category: syntax_category(&error),
                line: error.line(),
                column: error.column(),
            });
        }
    };
    Ok(WorkerDebugFlags {
        debug: read_flag(&object, "debug")?,
        detailed_debug: read_flag(&object, "detailed_debug")?,
    })
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
