use std::fs;
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};

use super::{MAX_FRAME_BYTES, OracleLimits, WebSocketCase};

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("invalid WebSocket transcript: {0}")]
    Parse(String),
    #[error("invalid WebSocket transcript: {0}")]
    Invalid(String),
    #[error("cannot read binary fixture: {0}")]
    Fixture(String),
}

pub fn decode_ws_case(bytes: &[u8], limits: &OracleLimits) -> Result<WebSocketCase, DecodeError> {
    let input_limit = limits
        .max_capture_bytes
        .checked_add(2_097_152)
        .ok_or_else(|| DecodeError::Invalid("transcript input limit overflow".to_owned()))?;
    if bytes.len() > input_limit {
        return Err(DecodeError::Invalid(
            "transcript input exceeds safety limit".to_owned(),
        ));
    }
    let text = std::str::from_utf8(bytes).map_err(|error| DecodeError::Parse(error.to_string()))?;
    let mut case: WebSocketCase =
        serde_saphyr::from_str(text).map_err(|error| DecodeError::Parse(error.to_string()))?;
    if case.kind != "websocket"
        || case.path.is_empty()
        || !(100..=599).contains(&case.expected_http_status)
    {
        return Err(DecodeError::Invalid(
            "invalid discriminator, path or handshake status".to_owned(),
        ));
    }
    if case.frames.len() > limits.max_frames {
        return Err(DecodeError::Invalid("frame count exceeds limit".to_owned()));
    }
    let mut traffic = 0usize;
    for (index, frame) in case.frames.iter_mut().enumerate() {
        if frame.text.is_some() == frame.binary.is_some() {
            return Err(DecodeError::Invalid(format!(
                "frame {index} needs exactly one payload"
            )));
        }
        let size = match &frame.text {
            Some(text) => text.len(),
            None => match &frame.binary {
                Some(path) => {
                    let bytes = binary_fixture(path)?;
                    let size = bytes.len();
                    frame.binary_data = Some(bytes);
                    size
                }
                None => 0,
            },
        };
        if size > limits.max_frame_bytes {
            return Err(DecodeError::Invalid(format!(
                "frame {index} exceeds size limit"
            )));
        }
        traffic = traffic
            .checked_add(size)
            .ok_or_else(|| DecodeError::Invalid("traffic size overflow".to_owned()))?;
    }
    if traffic > limits.max_capture_bytes {
        return Err(DecodeError::Invalid(
            "captured traffic exceeds limit".to_owned(),
        ));
    }
    if case.execution_seconds > limits.max_execution_seconds {
        return Err(DecodeError::Invalid("execution exceeds limit".to_owned()));
    }
    if case.expected_http_status != 101 && case.close.is_some() {
        return Err(DecodeError::Invalid(
            "pre-upgrade refusal cannot contain an accepted close".to_owned(),
        ));
    }
    if let Some(close) = &case.close {
        if close.initiator != "client" && close.initiator != "provider" {
            return Err(DecodeError::Invalid(
                "close initiator must be client or provider".to_owned(),
            ));
        }
    }
    Ok(case)
}

pub fn validate_binary_fixture(path: &Path) -> Result<(), DecodeError> {
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(DecodeError::Invalid(
            "binary fixture path escapes its root".to_owned(),
        ));
    }
    Ok(())
}

fn binary_fixture(path: &str) -> Result<Vec<u8>, DecodeError> {
    let relative = Path::new(path);
    validate_binary_fixture(relative)?;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/websocket-oracle");
    let canonical_root =
        fs::canonicalize(&root).map_err(|source| DecodeError::Fixture(source.to_string()))?;
    let candidate = canonical_root.join(relative);
    let canonical_path =
        fs::canonicalize(&candidate).map_err(|source| DecodeError::Fixture(source.to_string()))?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err(DecodeError::Invalid(
            "binary fixture resolves outside its root".to_owned(),
        ));
    }
    let file = fs::File::open(canonical_path)
        .map_err(|source| DecodeError::Fixture(source.to_string()))?;
    let limit = u64::try_from(MAX_FRAME_BYTES)
        .map_err(|source| DecodeError::Invalid(source.to_string()))?
        .checked_add(1)
        .ok_or_else(|| DecodeError::Invalid("frame limit overflow".to_owned()))?;
    let mut bytes = Vec::new();
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(|source| DecodeError::Fixture(source.to_string()))?;
    Ok(bytes)
}
