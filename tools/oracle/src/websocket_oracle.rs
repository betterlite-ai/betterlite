use serde::Deserialize;
use std::fs;
use std::io::{self, Read as _};
use std::path::{Component, Path, PathBuf};

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

pub fn direction_matches(expected: &[Frame], observed: &[Frame]) -> Result<(), String> {
    for direction in [Direction::ClientToProvider, Direction::ProviderToClient] {
        let expected_direction = expected
            .iter()
            .filter(|frame| frame.direction == direction)
            .collect::<Vec<_>>();
        let observed_direction = observed
            .iter()
            .filter(|frame| frame.direction == direction)
            .collect::<Vec<_>>();
        for (index, (left, right)) in expected_direction
            .iter()
            .zip(&observed_direction)
            .enumerate()
        {
            if left.action != right.action {
                return Err(format!("/{}/{index}/action", peer_name(direction)));
            }
            if left.text != right.text || left.binary != right.binary {
                return Err(changed_peer_path(direction, index, left));
            }
        }
        if expected_direction.len() != observed_direction.len() {
            let index = expected_direction.len().min(observed_direction.len());
            let frame = expected_direction
                .get(index)
                .or_else(|| observed_direction.get(index));
            if let Some(frame) = frame {
                return Err(changed_peer_path(direction, index, frame));
            }
        }
    }
    Ok(())
}

fn changed_peer_path(direction: Direction, index: usize, frame: &Frame) -> String {
    let payload = if frame.text.is_some() {
        "text"
    } else {
        "binary"
    };
    format!("/{}/frames/{index}/{payload}", peer_name(direction))
}

fn peer_name(direction: Direction) -> &'static str {
    match direction {
        Direction::ClientToProvider => "client",
        Direction::ProviderToClient => "provider",
    }
}

pub fn close_matches(expected: Option<&Close>, observed: Option<&Close>) -> Result<(), String> {
    let Some(expected_close) = expected else {
        return if observed.is_none() {
            Ok(())
        } else {
            Err("/close".to_owned())
        };
    };
    let Some(observed_close) = observed else {
        return Err("/close".to_owned());
    };
    if expected_close.initiator != observed_close.initiator {
        return Err("/close/initiator".to_owned());
    }
    if expected_close.code != observed_close.code {
        return Err("/close/code".to_owned());
    }
    if expected_close.reason != observed_close.reason {
        return Err("/close/reason".to_owned());
    }
    Ok(())
}

pub fn barrier_is_released(frame: &Frame, provider_observations: &[String]) -> bool {
    frame.barrier.as_ref().is_none_or(|barrier| {
        provider_observations
            .iter()
            .any(|observation| observation == barrier)
    })
}

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
        if let Some(binary) = &frame.binary {
            validate_binary_fixture(Path::new(binary))?;
        }
        let size = match &frame.text {
            Some(text) => text.len(),
            None => match &frame.binary {
                Some(path) => binary_fixture(path)?.len(),
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
        .map_err(|source: io::Error| DecodeError::Fixture(source.to_string()))?;
    Ok(bytes)
}

mod tests;
