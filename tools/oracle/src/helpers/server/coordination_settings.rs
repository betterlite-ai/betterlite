use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::coordination_redis_settings::projection::{
    Redaction, merge_over_saved, redact, scrub_credentials,
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
            Function::Redact => Ok(redact(settings, Redaction::Credentials)),
            Function::Merge => Ok(merge_over_saved(
                settings.unwrap_or(&input.settings),
                (!input.saved.is_null()).then_some(&input.saved),
            )),
            Function::Audit => Ok(redact(settings, Redaction::Audit)),
            Function::Scrub => Ok(json!({
                "message": scrub_credentials(&input.message, settings.unwrap_or(&input.settings)),
            })),
        }
    }
}
