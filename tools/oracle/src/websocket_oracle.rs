use serde::Deserialize;
use std::path::{Component, Path};

pub const MAX_FRAMES: usize = 1024;
pub const MAX_FRAME_BYTES: usize = 1_048_576;
pub const MAX_CAPTURE_BYTES: usize = 16_777_216;
pub const MAX_EXECUTION_SECONDS: u64 = 120;

#[derive(Debug, Clone, Copy)]
pub struct OracleLimits {
    pub max_frames: usize,
    pub max_frame_bytes: usize,
    pub max_capture_bytes: usize,
    pub max_execution_seconds: u64,
}

impl Default for OracleLimits {
    fn default() -> Self {
        Self {
            max_frames: MAX_FRAMES,
            max_frame_bytes: MAX_FRAME_BYTES,
            max_capture_bytes: MAX_CAPTURE_BYTES,
            max_execution_seconds: MAX_EXECUTION_SECONDS,
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WebSocketCase {
    pub kind: String,
    pub path: String,
    #[serde(default)]
    pub headers: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub requested_subprotocols: Vec<String>,
    pub expected_http_status: u16,
    pub negotiated_subprotocol: Option<String>,
    pub frames: Vec<Frame>,
    #[serde(default)]
    pub close: Option<Close>,
    #[serde(default)]
    pub execution_seconds: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub direction: Direction,
    pub action: Action,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub binary: Option<String>,
    #[serde(default)]
    pub barrier: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    ClientToProvider,
    ProviderToClient,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Send,
    Expect,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Close {
    pub initiator: String,
    pub code: u16,
    pub reason: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("invalid WebSocket transcript: {0}")]
    Parse(String),
    #[error("invalid WebSocket transcript: {0}")]
    Invalid(String),
}

pub fn decode_ws_case(bytes: &[u8], limits: &OracleLimits) -> Result<WebSocketCase, DecodeError> {
    let text = std::str::from_utf8(bytes).map_err(|error| DecodeError::Parse(error.to_string()))?;
    let case: WebSocketCase =
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
    for (index, frame) in case.frames.iter().enumerate() {
        if frame.text.is_some() == frame.binary.is_some() {
            return Err(DecodeError::Invalid(format!(
                "frame {index} needs exactly one payload"
            )));
        }
        let size = frame
            .text
            .as_ref()
            .map_or_else(|| frame.binary.as_ref().map_or(0, String::len), String::len);
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

mod tests;
