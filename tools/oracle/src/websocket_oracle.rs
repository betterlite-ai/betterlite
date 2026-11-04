use crate::websocket_driver::service::{Arguments, ServiceTag};
use serde::{Deserialize, Serialize};
use serde_with::{As, FromInto, Same};
use std::collections::BTreeMap;

use fixture::FixtureFrame;

pub const MAX_FRAMES: usize = 1024;
pub const MAX_FRAME_BYTES: usize = 0x0010_0000;
pub const MAX_CAPTURE_BYTES: usize = 0x0100_0000;
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

// Resolved by B1011/B528; B901 reads neither environment variable itself.
#[derive(Debug, Clone, Copy)]
pub struct RealtimeSettings {
    pub credential_timeout_seconds: f64,
    pub max_message_size_bytes: Option<usize>,
}

impl Default for RealtimeSettings {
    fn default() -> Self {
        Self {
            credential_timeout_seconds: 20.0,
            max_message_size_bytes: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct WebSocketBounds {
    pub limits: OracleLimits,
    pub settings: RealtimeSettings,
}

#[derive(Debug, Clone, Deserialize)]
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
    pub service: Option<ServiceTag>,
    #[serde(default)]
    pub driver: Driver,
    #[serde(
        default,
        deserialize_with = "As::<FromInto<Arguments<FixtureFrame>>>::deserialize"
    )]
    pub input: Arguments,
    pub path: String,
    pub headers: BTreeMap<String, String>,
    pub requested_subprotocols: Vec<String>,
    pub expected_http_status: u16,
    #[serde(with = "As::<Option<Same>>")]
    pub negotiated_subprotocol: Option<String>,
    #[serde(deserialize_with = "As::<Vec<FromInto<FixtureFrame>>>::deserialize")]
    pub frames: Vec<Frame>,
    #[serde(default)]
    pub close: Option<Close>,
    #[serde(default = "default_execution_seconds")]
    pub execution_seconds: u64,
    #[serde(default)]
    pub upstream_status: Option<u16>,
    #[serde(default)]
    pub route_model: Option<String>,
    #[serde(default)]
    pub route_guardrails: Vec<String>,
    #[serde(default)]
    pub authentication_detail: Option<String>,
    #[serde(default)]
    pub attempted_close_code: Option<u16>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RouteObservation {
    pub model: Option<String>,
    pub guardrails: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct AuthObservation {
    pub detail: Option<String>,
    pub attempted_close_code: Option<u16>,
}

fn first_mismatch(checks: &[(bool, &str)]) -> Result<(), String> {
    for &(equal, pointer) in checks {
        if !equal {
            return Err(pointer.to_owned());
        }
    }
    Ok(())
}

pub fn route_auth_matches(
    expected_route: &RouteObservation,
    observed_route: &RouteObservation,
    expected_auth: &AuthObservation,
    observed_auth: &AuthObservation,
) -> Result<(), String> {
    first_mismatch(&[
        (expected_route.model == observed_route.model, "/route/model"),
        (
            expected_route.guardrails == observed_route.guardrails,
            "/route/guardrails",
        ),
        (expected_auth.detail == observed_auth.detail, "/auth/detail"),
        (
            expected_auth.attempted_close_code == observed_auth.attempted_close_code,
            "/auth/attempted_close_code",
        ),
    ])
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub direction: Direction,
    pub action: Action,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub binary: Option<String>,
    #[serde(default)]
    pub binary_data: Option<Vec<u8>>,
    #[serde(default)]
    pub barrier: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    ClientToProvider,
    ProviderToClient,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Send,
    Expect,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Close {
    pub initiator: Initiator,
    pub code: u16,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Initiator {
    Client,
    Provider,
}

impl Initiator {
    #[must_use]
    pub const fn for_direction(sending: Direction) -> Self {
        match sending {
            Direction::ClientToProvider => Self::Client,
            Direction::ProviderToClient => Self::Provider,
        }
    }
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
                return Err(format!("/{}/frames/{index}/action", peer_name(direction)));
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
            if let Some(missing) = frame {
                return Err(changed_peer_path(direction, index, missing));
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

const fn peer_name(direction: Direction) -> &'static str {
    match direction {
        Direction::ClientToProvider => "client",
        Direction::ProviderToClient => "provider",
    }
}

pub fn close_matches(expected: Option<&Close>, observed: Option<&Close>) -> Result<(), String> {
    let (Some(expected_close), Some(observed_close)) = (expected, observed) else {
        return first_mismatch(&[(expected.is_none() == observed.is_none(), "/close")]);
    };
    first_mismatch(&[
        (
            expected_close.initiator == observed_close.initiator,
            "/close/initiator",
        ),
        (expected_close.code == observed_close.code, "/close/code"),
        (
            expected_close.reason == observed_close.reason,
            "/close/reason",
        ),
    ])
}

pub mod decode;
mod fixture;
mod tests;

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Driver {
    #[default]
    Gateway,
    Service,
}

const fn default_execution_seconds() -> u64 {
    MAX_EXECUTION_SECONDS
}
