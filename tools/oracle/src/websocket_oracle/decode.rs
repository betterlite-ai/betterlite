use std::fs::{self, File};
use std::io::{self, Read as _};
use std::path::{Component, Path, PathBuf};
use std::str::{self, Utf8Error};

use super::{
    Driver, Frame, MAX_CAPTURE_BYTES, MAX_EXECUTION_SECONDS, MAX_FRAME_BYTES, MAX_FRAMES,
    OracleLimits, WebSocketCase,
};

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("invalid WebSocket UTF-8: {0}")]
    Utf8(#[from] Utf8Error),
    #[error("invalid WebSocket YAML: {0}")]
    Parse(#[source] Box<serde_saphyr::Error>),
    #[error("invalid WebSocket transcript: {0}")]
    Invalid(String),
    #[error("cannot read binary fixture: {0}")]
    Fixture(#[from] io::Error),
}

pub fn decode_ws_case(bytes: &[u8], limits: &OracleLimits) -> Result<WebSocketCase, DecodeError> {
    if bytes.len()
        > limits
            .max_capture_bytes
            .min(MAX_CAPTURE_BYTES)
            .saturating_add(0x0020_0000)
    {
        return Err(DecodeError::Invalid(
            "transcript input exceeds safety limit".to_owned(),
        ));
    }
    let text = str::from_utf8(bytes)?;
    let mut case: WebSocketCase =
        serde_saphyr::from_str(text).map_err(|source| DecodeError::Parse(Box::new(source)))?;
    validate_case(&case, limits)?;
    let mut traffic = 0_usize;
    for frame in &mut case.frames {
        let size = resolve_frame(frame)?;
        if size > limits.max_frame_bytes || size > MAX_FRAME_BYTES {
            return Err(DecodeError::Invalid("frame exceeds size limit".to_owned()));
        }
        traffic = traffic.saturating_add(size);
    }
    if traffic > limits.max_capture_bytes.min(MAX_CAPTURE_BYTES) {
        return Err(DecodeError::Invalid(
            "captured traffic exceeds limit".to_owned(),
        ));
    }
    Ok(case)
}

fn validate_case(case: &WebSocketCase, limits: &OracleLimits) -> Result<(), DecodeError> {
    if case.kind != "websocket"
        || !case.path.starts_with('/')
        || !(100..=599).contains(&case.expected_http_status)
    {
        return Err(DecodeError::Invalid(
            "invalid discriminator, path or handshake status".to_owned(),
        ));
    }
    if case.frames.len() > limits.max_frames.min(MAX_FRAMES)
        || case.execution_seconds > limits.max_execution_seconds.min(MAX_EXECUTION_SECONDS)
    {
        return Err(DecodeError::Invalid(
            "frame count or execution exceeds limit".to_owned(),
        ));
    }
    if (case.driver == Driver::Gateway) == case.service.is_some() {
        return Err(DecodeError::Invalid(
            "driver and service selection disagree".to_owned(),
        ));
    }
    if let Some(close) = &case.close
        && (case.expected_http_status != 101
            || !["client", "provider"].contains(&close.initiator.as_str()))
    {
        return Err(DecodeError::Invalid(
            "invalid close initiator or pre-upgrade close".to_owned(),
        ));
    }
    Ok(())
}

fn resolve_frame(frame: &mut Frame) -> Result<usize, DecodeError> {
    match (&frame.text, &frame.binary) {
        (Some(text), None) => Ok(text.len()),
        (None, Some(path)) => {
            let data = binary_fixture(path)?;
            let size = data.len();
            frame.binary_data = Some(data);
            Ok(size)
        }
        (Some(_), Some(_)) | (None, None) => Err(DecodeError::Invalid(
            "frame needs exactly one payload".to_owned(),
        )),
    }
}

pub fn validate_binary_fixture(path: &Path) -> Result<(), DecodeError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
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
    let root = fs::canonicalize(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/websocket-oracle"),
    )?;
    let candidate = fs::canonicalize(root.join(relative))?;
    if !candidate.starts_with(&root) {
        return Err(DecodeError::Invalid(
            "binary fixture resolves outside its root".to_owned(),
        ));
    }
    let file = File::open(candidate)?;
    let mut bytes = Vec::new();
    file.take(0x0010_0001).read_to_end(&mut bytes)?;
    Ok(bytes)
}
