use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::mcp_identifier_resolution::{ServerNames, identifier_matches};
use serde::Deserialize;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    server: ServerInput,
    identifier: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServerInput {
    server_id: Option<String>,
    alias: Option<String>,
    server_name: Option<String>,
    name: Option<String>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let names = ServerNames {
            server_id: input.server.server_id.as_deref(),
            alias: input.server.alias.as_deref(),
            server_name: input.server.server_name.as_deref(),
            name: input.server.name.as_deref(),
        };
        let matches = identifier_matches(&names, &input.identifier);
        Ok(serde_json::json!({ "matches": matches }))
    }
}
