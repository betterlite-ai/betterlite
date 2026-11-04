use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use crate::websocket_oracle::{Close, Frame, WebSocketCase, close_matches, direction_matches};

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ServiceOperation {
    NativeResponsesRelay,
    ManagedResponsesTurns,
    RealtimeProbe,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceInput {
    pub operation: ServiceOperation,
    pub input: Value,
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
    #[error("service adapter {0:?} is not registered")]
    AdapterMissing(ServiceOperation),
    #[error("service adapter {0:?} is already registered")]
    DuplicateAdapter(ServiceOperation),
    #[error("service adapter failed: {0}")]
    Adapter(String),
}

pub struct ServiceRegistry {
    adapters: BTreeMap<ServiceOperation, fn(&Value) -> Result<Value, String>>,
}

impl ServiceRegistry {
    pub fn register(
        &mut self,
        operation: ServiceOperation,
        adapter: fn(&Value) -> Result<Value, String>,
    ) -> Result<(), DriverError> {
        if self.adapters.contains_key(&operation) {
            return Err(DriverError::DuplicateAdapter(operation));
        }
        self.adapters.insert(operation, adapter);
        Ok(())
    }

    pub fn dispatch(&self, input: &ServiceInput) -> Result<Value, DriverError> {
        let adapter = self
            .adapters
            .get(&input.operation)
            .ok_or(DriverError::AdapterMissing(input.operation))?;
        adapter(&input.input).map_err(DriverError::Adapter)
    }
}

impl Default for ServiceRegistry {
    fn default() -> Self {
        Self {
            adapters: BTreeMap::new(),
        }
    }
}
