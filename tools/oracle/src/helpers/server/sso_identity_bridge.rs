use std::collections::BTreeMap;

use betterlite_config::sso_identity_bridge::load_bridge_settings;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::sso_identity_bridge::microsoft_subject::{
    MicrosoftEndpoints, microsoft_interactive_subject,
};
use betterlite_server::sso_identity_bridge::provider::{
    active_sso_provider,
    id_jag_assertion_capture_gap,
    // ast-grep-ignore: no-long-names
    id_jag_assertion_capture_gap_at_startup as gap_at_startup,
};
use serde::Deserialize;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Provider(EnvironmentInput),
    Gap(EnvironmentInput),
    StartupGap(EnvironmentInput),
    Subject(SubjectInput),
}

#[derive(Debug, Deserialize)]
pub(crate) struct EnvironmentInput {
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SubjectInput {
    tenant: Option<String>,
    response_id: Option<String>,
    #[serde(default)]
    endpoints: EndpointsInput,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct EndpointsInput {
    authorization: Option<String>,
    token: Option<String>,
    userinfo: Option<String>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Provider(EnvironmentInput { environment }) => {
                let settings = load_bridge_settings(environment);
                Ok(serde_json::json!(active_sso_provider(&settings).value()))
            }
            Input::Gap(EnvironmentInput { environment }) => {
                let settings = load_bridge_settings(environment);
                Ok(serde_json::json!(id_jag_assertion_capture_gap(&settings)))
            }
            Input::StartupGap(EnvironmentInput { environment }) => {
                let settings = load_bridge_settings(environment);
                Ok(serde_json::json!(gap_at_startup(&settings)))
            }
            Input::Subject(subject) => {
                let endpoints = MicrosoftEndpoints {
                    authorization: subject.endpoints.authorization.as_deref(),
                    token: subject.endpoints.token.as_deref(),
                    userinfo: subject.endpoints.userinfo.as_deref(),
                };
                let projected = microsoft_interactive_subject(
                    subject.tenant.as_deref(),
                    subject.response_id.as_deref(),
                    &endpoints,
                );
                Ok(serde_json::json!(projected))
            }
        }
    }
}
