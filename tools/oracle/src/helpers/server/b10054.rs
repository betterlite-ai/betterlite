use std::collections::BTreeMap;

use betterlite_config::sections::settings::{JsonValue, Section, SettingsStore};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::proxy_config_state::supported_db_objects::should_load_db_object;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    general_settings: BTreeMap<String, JsonValue>,
    object_type: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut settings = SettingsStore::from(Section::GeneralSettings);
        settings.load_yaml(input.general_settings.clone());
        Ok(Value::Bool(should_load_db_object(
            &settings,
            &input.object_type,
        )))
    }
}
