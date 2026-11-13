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
impl crate::helpers::Helper for Op {
    type Input = Input;
    fn run(input: &Input) -> crate::helpers::HelperResult {
        let value = input.value.as_deref();
        let parsed = betterlite_config::env_integer::get_env_int(value, input.default);
        let range = match (input.minimum, input.maximum) {
            (Some(min), Some(max)) => betterlite_config::env_integer::get_env_int_in_range(
                "VALUE",
                value,
                input.default,
                min,
                max,
            ),
            _ => parsed,
        };
        Ok(
            json!({"int":parsed,"range":range,"optional":betterlite_config::env_integer::get_env_int_or_none(value)}),
        )
    }
}
