use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::coordination_redis_settings::projection::{
    merge_over_saved, redact_all_values, redact_credentials, scrub_credentials,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    Redact,
    Merge,
    Audit,
    Scrub,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    settings: Value,
    #[serde(default)]
    saved: Value,
    #[serde(default)]
    message: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let settings = (!input.settings.is_null()).then_some(&input.settings);
        match input.function {
            Function::Redact => Ok(redact_credentials(settings)),
            Function::Merge => Ok(merge_over_saved(
                settings.unwrap_or(&input.settings),
                (!input.saved.is_null()).then_some(&input.saved),
            )),
            Function::Audit => Ok(redact_all_values(settings)),
            Function::Scrub => Ok(json!({
                "message": scrub_credentials(&input.message, settings.unwrap_or(&input.settings)),
            })),
        }
    }
}
