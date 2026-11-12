pub(crate) mod assemble;
pub(crate) mod bridge;
pub(crate) mod convert;
pub(crate) mod finish;
pub(crate) mod headers;
pub(crate) mod messages;
pub(crate) mod params;
pub(crate) mod provider;
use super::args::{CompletionArgs, CompletionSettings, JsonObject};
use super::call::PreparedCall;
use super::ports::CompletionPorts;
use betterlite_core::message_utilities::validators::StopSequences;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_models::decisions::Json;
pub(crate) struct Preparation<'request> {
    pub args: &'request CompletionArgs,
    pub settings: &'request CompletionSettings,
    pub ports: &'request CompletionPorts<'request>,
    pub call: PreparedCall,
    pub kwargs: JsonObject,
    pub non_default: JsonObject,
    pub snapshot: CompletionArgs,
    pub tool_choice: Option<Json>,
    pub stop: Option<StopSequences>,
    pub thinking: Option<Json>,
    pub skip_bridge: bool,
    pub control: ControlOptions,
    pub acompletion: bool,
}
