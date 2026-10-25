use std::collections::BTreeMap;

use crate::context::SessionContext;
use crate::errors::HarnessError;
use crate::events::Event;
use crate::options::OptionsKind;
use crate::types::{Capabilities, Harness, JsonObject};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersistedDir {
    pub private_path: String,
    // under ~/.cache/litellm-harness, so a later session can resume the runtime's own conversation
    pub cache_path: String,
}

// relative paths are inside the session's private_dir
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HarnessSessionSetup {
    pub files: BTreeMap<String, Vec<u8>>,
    pub persisted_dirs: Vec<PersistedDir>,
    pub skills_dir: Option<String>,
    // values may contain {private_dir}
    pub env: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HarnessTurnRequest {
    pub argv: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub stdin: String,
    pub cwd: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HarnessTurnResponse {
    pub final_text: String,
    pub output_json: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct TurnInput<'turn> {
    pub setup: &'turn HarnessSessionSetup,
    pub private_dir: &'turn str,
    pub prompt: &'turn str,
    pub native_session_id: Option<&'turn str>,
}

pub trait HarnessConfig {
    type Options: OptionsKind;
    const HARNESS: Harness;
    // CLI runtimes call a per-session model endpoint, in-process ones call the model directly
    const USES_MODEL_ENDPOINT: bool = true;

    fn capabilities(&self) -> Capabilities;

    fn get_options<S>(&self, context: &SessionContext<S>) -> Result<Self::Options, HarnessError> {
        context.settings.options.as_ref().map_or_else(
            || Ok(Self::Options::default()),
            |options| {
                Self::Options::pick(options)
                    .cloned()
                    .ok_or(HarnessError::OptionsMismatch {
                        given: options.name(),
                        harness: Self::HARNESS,
                        expected: Self::Options::NAME,
                    })
            },
        )
    }

    fn validate_environment<S>(&self, context: &SessionContext<S>) -> Result<(), HarnessError> {
        self.get_options(context)?;
        Ok(())
    }
}

pub trait CliHarnessConfig: HarnessConfig {
    type StreamState;

    fn get_binary(&self) -> &'static str;
    fn get_install_hint(&self) -> &'static str;
    fn transform_session_setup<S>(
        &self,
        context: &SessionContext<S>,
        private_dir: &str,
    ) -> Result<HarnessSessionSetup, HarnessError>;
    fn transform_turn_request<S>(
        &self,
        context: &SessionContext<S>,
        turn: TurnInput<'_>,
    ) -> Result<HarnessTurnRequest, HarnessError>;
    fn create_stream_state(&self) -> Self::StreamState;
    fn transform_stream_line(&self, line: &JsonObject, state: &mut Self::StreamState)
    -> Vec<Event>;
    fn get_native_session_id(&self, state: &Self::StreamState) -> Option<String>;
    // a failed turn is HarnessError::TurnFailed, which the runtime maps to StopReason::RuntimeError
    fn transform_turn_response<S>(
        &self,
        context: &SessionContext<S>,
        state: &Self::StreamState,
        exit_code: i32,
        stderr_tail: &[String],
    ) -> Result<HarnessTurnResponse, HarnessError>;
}
