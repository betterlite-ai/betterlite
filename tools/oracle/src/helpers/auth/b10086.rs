use betterlite_auth::bearer_credentials::get_bearer_token;
use betterlite_auth::dev_auth_mode::{AuthModeFlags, AuthModeSwitch, is_no_auth_dev_mode};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Bearer {
        value: String,
    },
    DevMode {
        master_key: Option<String>,
        enable_jwt_auth: bool,
        enable_oauth2_auth: bool,
        enable_oauth2_proxy_auth: bool,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        match input {
            Input::Bearer { value } => Ok(bearer_answer(value)),
            Input::DevMode {
                master_key,
                enable_jwt_auth,
                enable_oauth2_auth,
                enable_oauth2_proxy_auth,
            } => {
                let flags = AuthModeFlags {
                    enable_jwt_auth: AuthModeSwitch(*enable_jwt_auth),
                    enable_oauth2_auth: AuthModeSwitch(*enable_oauth2_auth),
                    enable_oauth2_proxy_auth: AuthModeSwitch(*enable_oauth2_proxy_auth),
                };
                let key = master_key.clone().map(SecretString::from);
                Ok(json!(is_no_auth_dev_mode(key.as_ref(), &flags)))
            }
        }
    }
}

fn bearer_answer(value: &str) -> Value {
    let token = get_bearer_token(value);
    let strict = token.as_ref().map_or("", ExposeSecret::expose_secret);
    let permissive = token.as_ref().map_or(value, ExposeSecret::expose_secret);
    json!({"strict": strict, "permissive": permissive})
}
