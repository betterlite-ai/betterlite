use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

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
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub config: String,
    #[serde(default)]
    pub expected_upstream_query: Option<String>,
    #[serde(default)]
    pub service: Option<String>,
    #[serde(default)]
    pub input: Value,
    pub path: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub requested_subprotocols: Vec<String>,
    pub expected_http_status: u16,
    pub negotiated_subprotocol: Option<String>,
    pub frames: Vec<Frame>,
    #[serde(default)]
    pub close: Option<Close>,
    #[serde(default)]
    pub execution_seconds: u64,
    #[serde(default)]
    pub upstream_status: Option<u16>,
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
    #[serde(skip)]
    pub binary_data: Option<Vec<u8>>,
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
            if left.text != right.text || left.binary_data != right.binary_data {
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

mod decode;
pub use decode::{DecodeError, decode_ws_case, validate_binary_fixture};
mod tests;
