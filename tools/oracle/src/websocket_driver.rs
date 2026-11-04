use std::collections::BTreeMap;
use std::sync::{OnceLock, RwLock};

use serde::Deserialize;
use serde_json::Value;

use crate::websocket_oracle::{Close, Frame, WebSocketCase, close_matches, direction_matches};

mod close;
mod encode;
mod gateway;
mod payload;
pub(crate) mod provider;
mod socket;
pub use gateway::exchange as exchange_gateway;
pub(crate) use provider::start as start_provider;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceInput {
    pub service: ServiceTag,
    pub input: Value,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ServiceTag {
    NativeResponsesRelay,
    ManagedResponsesTurns,
    RealtimeProbe,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandshakeObservation {
    pub status: u16,
    pub negotiated_subprotocol: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptObservation {
    pub handshake: HandshakeObservation,
    pub frames: Vec<Frame>,
    pub close: Option<Close>,
}

pub fn compare_transcript(
    expected: &WebSocketCase,
    observed: &TranscriptObservation,
) -> Result<(), String> {
    if expected.expected_http_status != observed.handshake.status {
        return Err("/handshake/status".to_owned());
    }
    if expected.negotiated_subprotocol != observed.handshake.negotiated_subprotocol {
        return Err("/handshake/negotiated_subprotocol".to_owned());
    }
    direction_matches(&expected.frames, &observed.frames)?;
    close_matches(expected.close.as_ref(), observed.close.as_ref())
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DriverError {
    #[error("WebSocket transport failed: {0}")]
    Transport(String),
    #[error("service adapter {0:?} is not registered")]
    AdapterMissing(ServiceTag),
    #[error("service adapter {0:?} is already registered")]
    DuplicateAdapter(ServiceTag),
    #[error("service adapter failed: {0}")]
    Adapter(String),
}

pub type ServiceAdapter = fn(&Value) -> Result<Value, String>;

static SERVICE_ADAPTERS: OnceLock<RwLock<BTreeMap<ServiceTag, ServiceAdapter>>> = OnceLock::new();

pub fn register_service(tag: ServiceTag, adapter: ServiceAdapter) -> Result<(), DriverError> {
    let adapters = SERVICE_ADAPTERS.get_or_init(|| RwLock::new(BTreeMap::new()));
    let mut adapters = adapters
        .write()
        .map_err(|_| DriverError::Adapter("service registry lock poisoned".to_owned()))?;
    if adapters.contains_key(&tag) {
        return Err(DriverError::DuplicateAdapter(tag));
    }
    adapters.insert(tag, adapter);
    Ok(())
}

pub fn run_service(tag: ServiceTag, input: &Value) -> Result<Value, DriverError> {
    let adapters = SERVICE_ADAPTERS.get_or_init(|| RwLock::new(BTreeMap::new()));
    let registry = adapters
        .read()
        .map_err(|_| DriverError::Adapter("service registry lock poisoned".to_owned()))?;
    let adapter = registry
        .get(&tag)
        .copied()
        .ok_or(DriverError::AdapterMissing(tag))?;
    drop(registry);
    adapter(input).map_err(DriverError::Adapter)
}
