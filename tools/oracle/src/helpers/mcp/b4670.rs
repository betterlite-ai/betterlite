use betterlite_mcp::ceilings::agent::agent_capped_servers;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::json;

mod tests;

// _agent_capped_servers: the agent's own grants and its access group ceiling narrow the allowed server list
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    allowed: Vec<String>,
    agent_servers: Vec<String>,
    access_group_servers: Option<Vec<String>>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        Ok(json!(agent_capped_servers(
            &input.allowed,
            &input.agent_servers,
            input.access_group_servers.as_deref(),
        )))
    }
}
