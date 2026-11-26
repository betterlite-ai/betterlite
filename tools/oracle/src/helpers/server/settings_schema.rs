use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::settings_services::config::{SettingsConfigError, settings_block};
use betterlite_server::settings_services::error::SettingsSchemaError;
use betterlite_server::settings_services::projection::settings_with_schema;
use betterlite_server::settings_services::records::SettingsRecord;
use betterlite_server::settings_services::schema::{RootSchema, root_schema};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
enum Failure {
    #[error(transparent)]
    Schema(#[from] SettingsSchemaError),
    #[error(transparent)]
    Config(#[from] SettingsConfigError),
    #[error("the projected settings could not be serialized")]
    Serialize(#[from] serde_json::Error),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    settings_key: String,
    config: BTreeMap<String, JsonValue>,
    schema: JsonValue,
    defaults: BTreeMap<String, JsonValue>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        answer(input).or_value_error()
    }
}

// A stand-in caller record: the case's pinned schema, the config block over the class's defaults
struct CaseRecord {
    schema: RootSchema,
    values: BTreeMap<String, JsonValue>,
    defaults: BTreeMap<String, JsonValue>,
}

impl SettingsRecord for CaseRecord {
    fn schema(&self) -> &RootSchema {
        &self.schema
    }

    fn values(&self) -> BTreeMap<String, JsonValue> {
        self.values.clone()
    }

    fn field_default(&self, field_name: &str) -> Option<JsonValue> {
        self.defaults.get(field_name).cloned()
    }
}

fn answer(input: &Input) -> Result<Value, Failure> {
    let block = settings_block(&input.config, &input.settings_key)?;
    let mut values = input.defaults.clone();
    for (key, value) in block {
        values.insert(key, value);
    }
    let record = CaseRecord {
        schema: root_schema(&input.schema)?,
        values,
        defaults: input.defaults.clone(),
    };
    Ok(serde_json::to_value(settings_with_schema(&record))?)
}
