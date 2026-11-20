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
    TagDescriptor(answers::TagDescriptorCase),
    TagCreate(answers::TagCreateCase),
    TagResolution(answers::TagResolutionCase),
    TagPerKey(answers::TagPerKeyCase),
    McpPerKey(answers::McpPerKeyCase),
    McpPerTeam(answers::McpPerTeamCase),
    AgentDescriptors(answers::AgentCase),
    AgentResolution(answers::AgentResolutionCase),
    SessionResolution(answers::SessionCase),
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
