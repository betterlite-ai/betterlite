use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redact::setting_redaction::{
    dump_redacted_config, redact_param_for_logging, redact_secret_values, redact_setting_value,
};
use betterlite_redact::structured::Payload;

#[derive(serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    RedactSecretValues {
        #[serde(default)]
        value: Payload,
    },
    RedactParamForLogging {
        #[serde(default)]
        param_name: Option<String>,
        #[serde(default)]
        value: Payload,
    },
    RedactSettingValue {
        name: String,
        #[serde(default)]
        value: Payload,
        is_full_admin: bool,
    },
    DumpRedactedConfig {
        #[serde(default)]
        value: Option<Payload>,
        redact_all: bool,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let result = match input {
            Input::RedactSecretValues { value } => redact_secret_values(value),
            Input::RedactParamForLogging { param_name, value } => {
                redact_param_for_logging(param_name.as_deref(), value)
            }
            Input::RedactSettingValue {
                name,
                value,
                is_full_admin,
            } => redact_setting_value(name, value, *is_full_admin),
            Input::DumpRedactedConfig { value, redact_all } => {
                match dump_redacted_config(value.as_ref(), *redact_all) {
                    Some(text) => {
                        serde_json::from_str(&text).map_err(|source| invalid_dump(&source))?
                    }
                    None => Payload::Null,
                }
            }
        };
        Ok(serde_json::json!({ "result": result }))
    }
}

// the dumped text is serde_json's own output, so a parse failure is a broken build rather than case input
fn invalid_dump(source: &serde_json::Error) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: source.to_string(),
    })
}

mod tests;
