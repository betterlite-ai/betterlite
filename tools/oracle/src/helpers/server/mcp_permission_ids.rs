use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::mcp_permission_ids::cleanup::{
    dedupe_preserving_order,
    // ast-grep-ignore: no-long-names - the LiteLLM function this task owns
    drop_stale_object_permission_mcp_identifiers,
    // ast-grep-ignore: no-long-names - the LiteLLM function this task owns
    flatten_resolved_mcp_server_ids,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Dedupe {
        values: Vec<String>,
    },
    Cleanup {
        object_permission: JsonData,
        identifiers: BTreeMap<String, Vec<String>>,
    },
    Flatten {
        identifiers: BTreeMap<String, Vec<String>>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Dedupe { values } => Ok(serde_json::to_value(dedupe_preserving_order(values))?),
            Input::Cleanup {
                object_permission,
                identifiers,
            } => {
                let mut permission = object_permission.clone();
                let server_ids = identifier_sets(identifiers);
                // ast-grep-ignore: no-long-names - the LiteLLM function this task owns
                drop_stale_object_permission_mcp_identifiers(&mut permission, &server_ids)
                    .or_value_error()?;
                Ok(serde_json::to_value(permission)?)
            }
            Input::Flatten { identifiers } => {
                let server_ids = identifier_sets(identifiers);
                // ast-grep-ignore: no-long-names - the LiteLLM function this task owns
                let resolved = flatten_resolved_mcp_server_ids(&server_ids);
                Ok(serde_json::to_value(resolved)?)
            }
        }
    }
}

fn identifier_sets(
    identifiers: &BTreeMap<String, Vec<String>>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut sets = BTreeMap::new();
    for (identifier, server_ids) in identifiers {
        sets.insert(identifier.clone(), server_ids.iter().cloned().collect());
    }
    sets
}
