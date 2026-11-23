use betterlite_config::sections::env_credential_login::Settings;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::env_credential_login::is_enabled;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
struct Call {
    general_settings: Snapshot,
    sso_provider_ready: bool,
}

// LiteLLM types each setting as `bool | None`, so a null key reads exactly like an absent one
#[derive(Debug, Deserialize)]
struct Snapshot {
    #[serde(rename = "disable_env_credential_login")]
    env_login_disabled: Option<bool>,
    #[serde(rename = "disable_password_login_when_sso_enabled")]
    sso_blocks_password_login: Option<bool>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answers: Vec<bool> = input
            .calls
            .iter()
            .map(|call| {
                let settings = Settings {
                    env_login_disabled: call
                        .general_settings
                        .env_login_disabled
                        .unwrap_or_default(),
                    sso_blocks_password_login: call
                        .general_settings
                        .sso_blocks_password_login
                        .unwrap_or_default(),
                };
                is_enabled(&settings, call.sso_provider_ready)
            })
            .collect();
        serde_json::to_value(answers).map_err(|source| {
            Box::new(HelperError {
                class: "Output".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: source.to_string(),
            })
        })
    }
}
