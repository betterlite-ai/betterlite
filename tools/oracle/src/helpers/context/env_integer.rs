use betterlite_config::env_integer;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    value: Option<String>,
    default: i64,
    minimum: Option<i64>,
    maximum: Option<i64>,
}
pub(crate) struct Op;
impl Helper for Op {
    type Input = Input;
    fn run(input: &Input) -> HelperResult {
        let value = input.value.as_deref();
        let parsed = env_integer::get_env_int(value, input.default);
        let range = match (input.minimum, input.maximum) {
            (Some(min), Some(max)) => {
                env_integer::get_env_int_in_range("VALUE", value, input.default, min, max)
            }
            _ => parsed,
        };
        Ok(json!({"int":parsed,"range":range,"optional":env_integer::get_env_int_or_none(value)}))
    }
}
