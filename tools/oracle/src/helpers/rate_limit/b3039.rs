use serde::Deserialize;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

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
            Input::AgentResolution(case) => answers::agent_resolution(case),
            Input::SessionResolution(case) => answers::session_resolution(case),
        }
    }
}
