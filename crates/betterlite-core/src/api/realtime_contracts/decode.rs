use super::error::RealtimeContractError;
use super::records::{
    RealtimeClientSecretRequest, RealtimeClientSecretResponse, RealtimeExpiresAfter,
    RealtimeSessionConfig, TranscriptionSessionRequest, TranscriptionSessionResponse,
};
use betterlite_models::decisions::Json;
use serde::Serialize;
use serde_json::Deserializer;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RealtimeDecodeLimits {
    pub max_bytes: usize,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealtimeContractKind {
    Expiry,
    Session,
    ClientSecretRequest,
    ClientSecretResponse,
    TranscriptionRequest,
    TranscriptionResponse,
}
impl TryFrom<u8> for RealtimeContractKind {
    type Error = RealtimeContractError;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Expiry),
            2 => Ok(Self::Session),
            3 => Ok(Self::ClientSecretRequest),
            4 => Ok(Self::ClientSecretResponse),
            5 => Ok(Self::TranscriptionRequest),
            6 => Ok(Self::TranscriptionResponse),
            _ => Err(RealtimeContractError::UnknownKind(value)),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum RealtimeContract {
    Expiry(RealtimeExpiresAfter),
    Session(RealtimeSessionConfig),
    ClientSecretRequest(RealtimeClientSecretRequest),
    ClientSecretResponse(RealtimeClientSecretResponse),
    TranscriptionRequest(TranscriptionSessionRequest),
    TranscriptionResponse(TranscriptionSessionResponse),
}
pub fn decode_realtime_frame(
    bytes: &[u8],
    limits: RealtimeDecodeLimits,
) -> Result<Json, RealtimeContractError> {
    check_size(bytes, limits)?;
    Ok(serde_json::from_slice(bytes)?)
}
pub fn decode_realtime_contract(
    bytes: &[u8],
    kind: RealtimeContractKind,
    limits: RealtimeDecodeLimits,
) -> Result<RealtimeContract, RealtimeContractError> {
    check_size(bytes, limits)?;
    let mut decoder = Deserializer::from_slice(bytes);
    let contract = match kind {
        RealtimeContractKind::Expiry => {
            RealtimeContract::Expiry(serde_path_to_error::deserialize(&mut decoder)?)
        }
        RealtimeContractKind::Session => {
            RealtimeContract::Session(serde_path_to_error::deserialize(&mut decoder)?)
        }
        RealtimeContractKind::ClientSecretRequest => {
            RealtimeContract::ClientSecretRequest(serde_path_to_error::deserialize(&mut decoder)?)
        }
        RealtimeContractKind::ClientSecretResponse => {
            RealtimeContract::ClientSecretResponse(serde_path_to_error::deserialize(&mut decoder)?)
        }
        RealtimeContractKind::TranscriptionRequest => {
            RealtimeContract::TranscriptionRequest(serde_path_to_error::deserialize(&mut decoder)?)
        }
        RealtimeContractKind::TranscriptionResponse => {
            RealtimeContract::TranscriptionResponse(serde_path_to_error::deserialize(&mut decoder)?)
        }
    };
    decoder.end().map_err(RealtimeContractError::Json)?;
    Ok(contract)
}
const fn check_size(
    bytes: &[u8],
    limits: RealtimeDecodeLimits,
) -> Result<(), RealtimeContractError> {
    if bytes.len() > limits.max_bytes {
        return Err(RealtimeContractError::JsonTooLarge {
            observed: bytes.len(),
            limit: limits.max_bytes,
        });
    }
    Ok(())
}
