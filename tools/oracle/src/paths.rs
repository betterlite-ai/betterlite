use std::env;
use std::fs;
use std::net::TcpListener;
use std::num::ParseIntError;
use std::path::PathBuf;
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

use crate::OracleError;

mod tests;

#[derive(Debug, Error)]
pub enum PortError {
    #[error("invalid /proc/sys/net/ipv4/ip_local_port_range: {0}")]
    Range(#[source] ParseIntError),
    #[error("no free gateway port on 127.0.0.1 in 10000..{end}")]
    Unavailable { end: u16 },
}

pub(crate) fn artifact_root() -> Result<PathBuf, OracleError> {
    let root = env::current_dir().map_err(|source| OracleError::Filesystem {
        path: PathBuf::from("."),
        source,
    })?;
    let path = root.join("target/oracle");
    fs::create_dir_all(&path).map_err(|source| OracleError::Filesystem {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

pub(crate) fn free_port() -> Result<u16, OracleError> {
    let path = PathBuf::from("/proc/sys/net/ipv4/ip_local_port_range");
    let range =
        fs::read_to_string(&path).map_err(|source| OracleError::Filesystem { path, source })?;
    port_in_range(&range, process::id()).map_err(OracleError::GatewayPort)
}

fn port_in_range(range: &str, process_id: u32) -> Result<u16, PortError> {
    let end = range
        .split_whitespace()
        .next()
        .unwrap_or("")
        .parse::<u16>()
        .map_err(PortError::Range)?;
    let offset = process_id
        .checked_rem(u32::from(end.saturating_sub(10000)))
        .ok_or(PortError::Unavailable { end })?;
    let start = u16::try_from(offset)
        .ok()
        .and_then(|value| 10000_u16.checked_add(value))
        .ok_or(PortError::Unavailable { end })?;
    // Port-zero mock listeners must not reclaim the port during Python startup.
    for port in (start..end).chain(10000..start) {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }
    Err(PortError::Unavailable { end })
}

pub(crate) fn unique_id() -> Result<String, OracleError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(OracleError::Clock)?;
    Ok(format!("{}-{}", process::id(), elapsed.as_nanos()))
}

pub(crate) fn safe_name(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "case".to_owned()
    } else {
        sanitized
    }
}
