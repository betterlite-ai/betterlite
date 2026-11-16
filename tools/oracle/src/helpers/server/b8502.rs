use betterlite_models::decisions::Extra;
use betterlite_server::cache_settings::redact::redact_settings;
use betterlite_server::cache_settings_update::manager::params_equal;
use serde::Deserialize;
use serde_json::{Map, Value};

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    CacheParamsEqual {
        params1: Extra,
        params2: Extra,
    },
    RedactSettings {
        settings: Option<Map<String, Value>>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let output = match input {
            Input::CacheParamsEqual { params1, params2 } => Value::Bool(params_equal(params1, params2)),
            Input::RedactSettings { settings } => {
                let empty = Map::new();
                Value::Object(redact_settings(settings.as_ref().unwrap_or(&empty)))
            }
        };
        Ok(output)
    }
}
