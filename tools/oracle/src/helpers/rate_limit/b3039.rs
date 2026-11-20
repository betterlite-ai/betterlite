use serde::Deserialize;
use serde_json::json;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_rate_limit::identity_descriptors::agent_resolution::resolved_agent_id;
use betterlite_rate_limit::identity_descriptors::session_resolution::session_id_from_data;

mod answers;
mod fakes;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    // boxed so every variant carries a pointer, well within the 3x variant_size_differences enforces
    TagDescriptor(Box<answers::TagDescriptorCase>),
    TagCreate(Box<answers::TagCreateCase>),
    TagResolution(Box<answers::TagResolutionCase>),
    TagPerKey(Box<answers::TagPerKeyCase>),
    McpPerKey(Box<answers::McpPerKeyCase>),
    McpPerTeam(Box<answers::McpPerTeamCase>),
    AgentDescriptors(Box<answers::AgentCase>),
    AgentResolution(Box<answers::AgentResolutionCase>),
    SessionResolution(Box<answers::SessionCase>),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::TagDescriptor(case) => answers::tag_descriptor(case),
            Input::TagCreate(case) => answers::tag_create(case),
            Input::TagResolution(case) => answers::tag_resolution(case),
            Input::TagPerKey(case) => answers::tag_per_key(case),
            Input::McpPerKey(case) => answers::mcp_per_key(case),
            Input::McpPerTeam(case) => answers::mcp_per_team(case),
            Input::AgentDescriptors(case) => answers::agent_descriptors(case),
            Input::AgentResolution(case) => {
                let agent_id = resolved_agent_id(case.key_agent_id.as_deref(), &case.data);
                Ok(json!({"agent_id": agent_id}))
            }
            Input::SessionResolution(case) => {
                let session_id = session_id_from_data(&case.data);
                Ok(json!({"session_id": session_id}))
            }
        }
    }
}
