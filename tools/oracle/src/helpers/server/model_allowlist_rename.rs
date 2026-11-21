use betterlite_server::model_allowlist_rename::cache::cache_keys;
use betterlite_server::model_allowlist_rename::statement::{AllowlistKind, TouchedRow};
use serde::Deserialize;
use serde_json::json;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    kind: AllowlistKind,
    object_id: String,
    #[serde(default)]
    team_alias: Option<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let row = TouchedRow {
            kind: input.kind,
            object_id: input.object_id.clone(),
            team_alias: input.team_alias.clone(),
        };
        Ok(json!({ "keys": cache_keys(&row) }))
    }
}
