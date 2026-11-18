use std::iter::once;

use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_core::api::realtime_contracts::transport::RealtimeMessage;
use betterlite_models::decisions::Json;
use serde::Deserialize;

pub(crate) const MAX_OPERATIONS: usize = 32;
pub(crate) const MAX_FRAMES: usize = 64;
pub(crate) const MAX_TEXT_BYTES: usize = 16 * 1024;
pub(crate) const MAX_CAPTURE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Frame {
    Text { text: String },
    Binary { bytes: Vec<u8> },
}

impl Frame {
    pub(crate) fn message(self) -> RealtimeMessage {
        match self {
            Self::Text { text } => RealtimeMessage::Text(text),
            Self::Binary { bytes } => RealtimeMessage::Binary(bytes),
        }
    }

    const fn size(&self) -> usize {
        match self {
            Self::Text { text } => text.len(),
            Self::Binary { bytes } => bytes.len(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ProviderDocument {
    pub(crate) requires_session_configuration: bool,
    pub(crate) setup: Vec<Json>,
    pub(crate) content: Vec<Json>,
    pub(crate) transforms: Vec<Vec<Frame>>,
    pub(crate) fail_transform: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct GuardrailDocument {
    pub(crate) custom: bool,
    pub(crate) runs_for: Vec<GuardrailEventHooks>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Operation {
    Send { text: String },
    Event { text: String },
    Disable,
    Cache { text: String },
}

impl Operation {
    const fn size(&self) -> usize {
        match self {
            Self::Send { text } | Self::Event { text } | Self::Cache { text } => text.len(),
            Self::Disable => 0,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Input {
    pub(crate) model: String,
    pub(crate) force_transcription_model: Option<String>,
    pub(crate) client_wants_beta: bool,
    pub(crate) backend_uses_beta_protocol: Option<bool>,
    pub(crate) provider: Option<ProviderDocument>,
    pub(crate) guardrails: Vec<GuardrailDocument>,
    pub(crate) backend_failures: Vec<usize>,
    pub(crate) client_failures: Vec<usize>,
    pub(crate) operations: Vec<Operation>,
}

impl Input {
    pub(crate) fn check(&self) -> Result<(), String> {
        let batches = self
            .provider
            .iter()
            .flat_map(|provider| &provider.transforms);
        let frames = batches.clone().flatten();
        let sizes = once(self.model.len())
            .chain(self.force_transcription_model.iter().map(String::len))
            .chain(self.operations.iter().map(Operation::size))
            .chain(frames.clone().map(Frame::size));
        if self.operations.len() > MAX_OPERATIONS
            || self.guardrails.len() > MAX_FRAMES
            || self.backend_failures.len() > MAX_FRAMES
            || self.client_failures.len() > MAX_FRAMES
            || batches.count() > MAX_OPERATIONS
            || frames.count() > MAX_FRAMES
            || sizes.clone().any(|size| size > MAX_TEXT_BYTES)
            || sizes.fold(0_usize, usize::saturating_add) > MAX_CAPTURE_BYTES
        {
            return Err(String::from("the scripted sender input is past its limits"));
        }
        Ok(())
    }
}
