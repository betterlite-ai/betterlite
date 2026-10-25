use std::collections::BTreeMap;

use crate::context::SessionContext;
use crate::errors::HarnessError;
use crate::options::{
    ClaudeCodeOptions, CodexOptions, DeepAgentsOptions, HarnessOptions, OpenCodeOptions,
};
use crate::types::{Capabilities, Event, Harness};

pub trait HarnessOption: Clone + Default + Send + Sync + 'static {
    fn extract(options: &HarnessOptions) -> Option<Self>;
    fn type_name() -> &'static str;
}
macro_rules! option_impl {
    ($ty:ty, $variant:ident) => {
        impl HarnessOption for $ty {
            fn extract(options: &HarnessOptions) -> Option<Self> {
                if let HarnessOptions::$variant(value) = options {
                    Some(value.clone())
                } else {
                    None
                }
            }
            fn type_name() -> &'static str {
                std::any::type_name::<Self>()
            }
        }
    };
}
option_impl!(ClaudeCodeOptions, ClaudeCode);
option_impl!(CodexOptions, Codex);
option_impl!(OpenCodeOptions, OpenCode);
option_impl!(DeepAgentsOptions, DeepAgents);

pub struct HarnessSessionSetup {
    pub files: BTreeMap<String, Vec<u8>>,
    pub persisted_dirs: Vec<(String, String)>,
    pub skills_dir: Option<String>,
    pub env: BTreeMap<String, String>,
}
pub struct HarnessTurnRequest {
    pub argv: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub stdin: String,
    pub cwd: Option<String>,
}
pub struct HarnessTurnResponse {
    pub final_text: String,
    pub output_json: Option<String>,
}

pub trait BaseHarnessConfig: Send + Sync {
    type Options: HarnessOption;
    fn harness(&self) -> Harness;
    fn capabilities(&self) -> &Capabilities;
    fn get_options(&self, context: &SessionContext) -> Result<Self::Options, HarnessError> {
        context.options.as_ref().map_or_else(
            || Ok(Self::Options::default()),
            |options| {
                Self::Options::extract(options).ok_or_else(|| HarnessError::OptionsMismatch {
                    actual: options_name(options).to_owned(),
                    expected: Self::Options::type_name().to_owned(),
                    harness: self.harness().name().to_owned(),
                })
            },
        )
    }
    fn validate_environment(&self, context: &SessionContext) -> Result<(), HarnessError> {
        self.get_options(context).map(|_| ())
    }
}
pub trait HarnessConfigView: Send + Sync {
    fn harness(&self) -> Harness;
}
impl<T: BaseHarnessConfig> HarnessConfigView for T {
    fn harness(&self) -> Harness {
        BaseHarnessConfig::harness(self)
    }
}
pub trait BaseCLIHarnessConfig: BaseHarnessConfig {
    type StreamState: Send + 'static;
    fn get_binary(&self) -> String;
    fn get_install_hint(&self) -> String;
    fn transform_session_setup(
        &self,
        context: &SessionContext,
        private_dir: String,
    ) -> HarnessSessionSetup;
    fn transform_turn_request(
        &self,
        context: &SessionContext,
        setup: &HarnessSessionSetup,
        private_dir: String,
        prompt: String,
        native_session_id: Option<String>,
    ) -> HarnessTurnRequest;
    fn create_stream_state(&self) -> Self::StreamState;
    fn transform_stream_line(
        &self,
        line: &BTreeMap<String, serde_json::Value>,
        state: &mut Self::StreamState,
    ) -> Vec<Event>;
    fn get_native_session_id(&self, state: &Self::StreamState) -> Option<String>;
    fn transform_turn_response(
        &self,
        context: &SessionContext,
        state: &Self::StreamState,
        exit_code: i32,
        stderr_tail: Vec<String>,
    ) -> Result<HarnessTurnResponse, HarnessError>;
}

pub fn options_name(options: &HarnessOptions) -> &'static str {
    match options {
        HarnessOptions::ClaudeCode(_) => std::any::type_name::<ClaudeCodeOptions>(),
        HarnessOptions::Codex(_) => std::any::type_name::<CodexOptions>(),
        HarnessOptions::OpenCode(_) => std::any::type_name::<OpenCodeOptions>(),
        HarnessOptions::DeepAgents(_) => std::any::type_name::<DeepAgentsOptions>(),
    }
}
