use betterlite_server::cache_settings::merge::merge_over_saved;
use betterlite_server::cache_settings::parse::parse_stored_settings;
use betterlite_server::cache_settings::precedence::resolve_cache_url_precedence;
use betterlite_server::cache_settings::redact::{
    is_credential_field, redact_credentials, redact_settings,
};
use betterlite_server::cache_settings::reuse::saved_secret_is_reusable;
use betterlite_server::cache_settings::target::{has_connection_target, target_repr};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    UrlPrecedence {
        settings: Map<String, Value>,
    },
    ParseStored {
        value: Value,
    },
    RedactCredentials {
        settings: Map<String, Value>,
    },
    CredentialField {
        key: String,
    },
    HasTarget {
        value: Value,
    },
    TargetRepr {
        value: Value,
    },
    Reusable {
        incoming: Map<String, Value>,
        saved: Map<String, Value>,
    },
    Merge {
        incoming: Map<String, Value>,
        saved: Map<String, Value>,
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
            Input::UrlPrecedence { settings } => {
                Value::Object(resolve_cache_url_precedence(settings))
            }
            Input::ParseStored { value } => {
                Value::Object(parse_stored_settings(value).map_err(|error| parse_error(&error))?)
            }
            Input::RedactCredentials { settings } => Value::Object(redact_credentials(settings)),
            Input::CredentialField { key } => Value::Bool(is_credential_field(key)),
            Input::HasTarget { value } => Value::Bool(has_connection_target(value)),
            Input::TargetRepr { value } => Value::String(target_repr(value)),
            Input::Reusable { incoming, saved } => {
                Value::Bool(saved_secret_is_reusable(incoming, saved))
            }
            Input::Merge { incoming, saved } => Value::Object(merge_over_saved(incoming, saved)),
            Input::RedactSettings { settings } => Value::Object(redact_settings(settings.as_ref())),
        };
        Ok(output)
    }
}

fn parse_error(
    error: &betterlite_server::cache_settings::parse::ParseStoredSettingsError,
) -> Box<HelperError> {
    Box::new(HelperError {
        class: "JSONDecodeError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
