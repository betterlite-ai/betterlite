use betterlite_server::cache_activity_routes::{CACHE_ACTIVITY_PATH, DOCUMENTED_IN_OPENAPI};
use serde::Deserialize;
use serde_json::json;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    path: String,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        let documented = match input.path.as_str() {
            CACHE_ACTIVITY_PATH => DOCUMENTED_IN_OPENAPI,
            _ => false,
        };
        Ok(json!({"path": input.path, "documented": documented}))
    }
}
