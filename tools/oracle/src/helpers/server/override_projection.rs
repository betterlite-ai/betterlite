use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::config_override_support::audit::redact_config;
use betterlite_server::config_override_support::decode::decode_stored_override;
use betterlite_server::config_override_support::environment::project_environment;
use betterlite_server::config_override_support::masking::mask_sensitive_fields;
use betterlite_server::config_override_support::models::{OverrideKind, field_schema};
use serde::Deserialize;
use serde_json::{Map as JsonMap, Value};

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    Mask,
    Audit,
    Schema,
    Project,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    kind: OverrideKind,
    #[serde(default)]
    config: Value,
    #[serde(default)]
    raw: Value,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function {
            Function::Mask => {
                let config: JsonMap<String, Value> =
                    serde_json::from_value(input.config.clone()).or_value_error()?;
                serde_json::to_value(mask_sensitive_fields(input.kind, &config)).or_value_error()
            }
            Function::Audit => {
                let config: Option<JsonMap<String, Value>> = if input.config.is_null() {
                    None
                } else {
                    Some(serde_json::from_value(input.config.clone()).or_value_error()?)
                };
                serde_json::to_value(redact_config(config.as_ref())).or_value_error()
            }
            Function::Schema => serde_json::to_value(field_schema(input.kind)).or_value_error(),
            Function::Project => {
                let parsed = decode_stored_override(&input.raw, input.kind).or_value_error()?;
                serde_json::to_value(project_environment(input.kind, &parsed).changes)
                    .or_value_error()
            }
        }
    }
}
