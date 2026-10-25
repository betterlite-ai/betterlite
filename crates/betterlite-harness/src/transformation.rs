use std::collections::BTreeMap;

use crate::context::SessionContext;
use crate::errors::{HarnessError, OptionsMismatch};
use crate::options::HarnessOptions;
use crate::types::{Capabilities, Event, Harness};

pub fn event_list(events: Vec<Event>) -> Vec<Event> {
    let result = events;
    result
}
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
pub struct HarnessTurnError(pub String);
pub trait BaseHarnessConfig: Send + Sync {
    fn harness(&self) -> Harness;
    fn capabilities(&self) -> &Capabilities;
    fn options_type(&self) -> &'static str;
    fn get_options(&self, context: &SessionContext) -> Result<HarnessOptions, OptionsMismatch>;
    fn validate_environment(&self, context: &SessionContext) -> Result<(), HarnessError> {
        self.get_options(context)
            .map(|_| ())
            .map_err(|error| HarnessError(error.0))
    }
}
pub trait BaseCLIHarnessConfig: BaseHarnessConfig {
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
    fn create_stream_state(&self) -> serde_json::Value;
    fn transform_stream_line(
        &self,
        line: &BTreeMap<String, serde_json::Value>,
        state: &mut serde_json::Value,
    ) -> Vec<Event>;
    fn get_native_session_id(&self, state: &serde_json::Value) -> Option<String>;
    fn transform_turn_response(
        &self,
        context: &SessionContext,
        state: &serde_json::Value,
        exit_code: i32,
        stderr_tail: Vec<String>,
    ) -> Result<HarnessTurnResponse, HarnessTurnError>;
}
