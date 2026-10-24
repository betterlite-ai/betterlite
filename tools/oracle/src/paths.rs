use std::env;
use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::OracleError;

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
    let port = TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| listener.local_addr())
        .map_err(|source| OracleError::Filesystem {
            path: PathBuf::from("127.0.0.1:0"),
            source,
        })?;
    Ok(port.port())
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
