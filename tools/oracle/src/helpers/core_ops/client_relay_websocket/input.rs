use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use serde::Deserialize;

pub(crate) const MAX_FRAMES: usize = 32;
pub(crate) const MAX_TEXT_BYTES: usize = 16 * 1024;
pub(crate) const MAX_TOTAL_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mode {
    #[default]
    Ack,
    Lifetime,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CloseDocument {
    pub(crate) code: u16,
    pub(crate) reason: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GuardrailDocument {
    pub(crate) hooks: Vec<GuardrailEventHooks>,
    pub(crate) block_on: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SetupDocument {
    pub(crate) pending: Vec<String>,
    pub(crate) backend_setup_complete: Option<bool>,
    pub(crate) flushing: bool,
    pub(crate) turn_detection_sent: bool,
    pub(crate) pending_guardrail_message: Option<String>,
    pub(crate) session_configuration_request: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DirectionWait {
    None,
    Cancel,
    Signal,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ClientOutcome {
    ClientDisconnected,
    BackendClosed,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BackendOutcome {
    Close,
    Error,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LifetimeDocument {
    pub(crate) client_wait: DirectionWait,
    #[serde(default)]
    pub(crate) client_signals: bool,
    pub(crate) client_outcome: ClientOutcome,
    pub(crate) backend_wait: DirectionWait,
    #[serde(default)]
    pub(crate) backend_signals: bool,
    pub(crate) backend_outcome: BackendOutcome,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) mode: Mode,
    pub(crate) messages: Vec<String>,
    pub(crate) setup: SetupDocument,
    pub(crate) guardrails: Vec<GuardrailDocument>,
    pub(crate) beta_backend: bool,
    pub(crate) close: Option<CloseDocument>,
    pub(crate) lifetime: Option<LifetimeDocument>,
    pub(crate) client_send_failures: Vec<usize>,
    pub(crate) client_close_failures: Vec<usize>,
}

impl Input {
    pub(crate) fn check(&self) -> Result<(), String> {
        let sizes = self
            .messages
            .iter()
            .map(String::len)
            .chain(self.setup.pending.iter().map(String::len));
        if self.messages.len() > MAX_FRAMES
            || self.setup.pending.len() > MAX_FRAMES
            || sizes.clone().any(|size| size > MAX_TEXT_BYTES)
            || sizes.fold(0_usize, usize::saturating_add) > MAX_TOTAL_BYTES
        {
            return Err(String::from(
                "the scripted client relay input is past its limits",
            ));
        }
        if self.mode == Mode::Lifetime && (self.lifetime.is_none() || self.close.is_none()) {
            return Err(String::from(
                "a lifetime run needs both a lifetime script and a close document",
            ));
        }
        Ok(())
    }
}
