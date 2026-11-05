use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use thiserror::Error;

use crate::prisma_airs_masking::ScanResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Allow,
    Block,
}

// LiteLLM marks these on the result dict as _always_block and _is_transient.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    Permanent,
    Transient,
}

#[derive(Debug, Error)]
pub enum VerdictError {
    #[error("the scan response is not JSON: {0}")]
    NotJson(#[source] serde_json::Error),
    #[error("the scan response has an unexpected shape: {0}")]
    Shape(#[source] serde_json::Error),
}

#[derive(Debug, Deserialize)]
struct VerdictWire {
    action: Action,
    category: Option<String>,
    scan_id: Option<String>,
    error: Option<String>,
}

#[derive(Debug)]
pub struct ScanVerdict {
    pub action: Action,
    pub category: Option<String>,
    pub scan_id: Option<String>,
    pub error: Option<String>,
    pub masks: ScanResult,
    pub failure: Option<Failure>,
    // Every other field AIRS sent, kept as sent so the client-visible error detail can echo it.
    pub extra: IndexMap<String, Box<RawValue>>,
}

// The part of a verdict that goes into the standard logging record.
#[derive(Debug, Clone)]
pub struct VerdictSummary {
    pub action: Action,
    pub category: Option<String>,
    pub failure: Option<Failure>,
    pub extra: IndexMap<String, Box<RawValue>>,
}

impl ScanVerdict {
    #[must_use]
    pub fn summary(&self) -> VerdictSummary {
        VerdictSummary {
            action: self.action,
            category: self.category.clone(),
            failure: self.failure,
            extra: self.extra.clone(),
        }
    }

    #[must_use]
    pub fn synthetic(action: Action, category: &str, failure: Option<Failure>) -> Self {
        Self {
            action,
            category: Some(category.to_owned()),
            scan_id: None,
            error: None,
            masks: ScanResult {
                prompt_masked_data: None,
                response_masked_data: None,
            },
            failure,
            extra: IndexMap::new(),
        }
    }

    // Parses an AIRS response body; LiteLLM's "action" not in result check is the missing-action case.
    pub fn from_body(body: &[u8]) -> Result<Self, VerdictError> {
        serde_json::from_slice::<Box<RawValue>>(body).map_err(VerdictError::NotJson)?;
        let wire: VerdictWire = serde_json::from_slice(body).map_err(VerdictError::Shape)?;
        let masks: ScanResult = serde_json::from_slice(body).map_err(VerdictError::Shape)?;
        let mut extra: IndexMap<String, Box<RawValue>> =
            serde_json::from_slice(body).map_err(VerdictError::Shape)?;
        extra.shift_remove("action");
        extra.shift_remove("category");
        Ok(Self {
            action: wire.action,
            category: wire.category,
            scan_id: wire.scan_id,
            error: wire.error,
            masks,
            failure: None,
            extra,
        })
    }
}
