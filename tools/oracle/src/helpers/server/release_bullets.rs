use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::release_info::bullets::count_release_bullets;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    body: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let counts = count_release_bullets(&input.body);
        Ok(serde_json::json!({
            "new_features": counts.new_features,
            "bug_fixes": counts.bug_fixes,
            "other_updates": counts.other_updates,
        }))
    }
}
