use std::collections::BTreeSet;
use std::future::{Future, ready};

use betterlite_config::sections::copilot_chat::Settings;
use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::{CapabilityPredicates, CapabilityRequest};
use betterlite_openai::copilot::api_key::error::ApiKeyError;
use betterlite_openai::copilot::device_auth::error::DeviceAuthError;
use betterlite_openai::openai_base_config::{CatalogCapabilities, Headers};
use secrecy::SecretString;

use crate::api::copilot_chat::GithubCopilotConfig;
use crate::api::copilot_chat::credentials::CopilotCredentials;

#[derive(Default)]
pub(super) enum KeyOutcome {
    #[default]
    NoToken,
    Token(&'static str),
    Refresh,
    Login {
        status: u16,
    },
}

#[derive(Default)]
pub(super) struct FakeCredentials {
    pub(super) base: Option<&'static str>,
    pub(super) key: KeyOutcome,
    pub(super) key_calls: usize,
    pub(super) base_calls: usize,
}

impl CopilotCredentials for FakeCredentials {
    fn api_base(&mut self) -> impl Future<Output = Option<String>> + Send {
        self.base_calls += 1;
        ready(self.base.map(String::from))
    }

    fn api_key(
        &mut self,
    ) -> impl Future<Output = Result<Option<SecretString>, ApiKeyError>> + Send {
        self.key_calls += 1;
        ready(match &self.key {
            KeyOutcome::NoToken => Ok(None),
            KeyOutcome::Token(text) => Ok(Some(SecretString::from(String::from(*text)))),
            KeyOutcome::Refresh => Err(ApiKeyError::Refresh),
            KeyOutcome::Login { status } => {
                Err(ApiKeyError::AccessToken(DeviceAuthError::ApiKey {
                    status: *status,
                }))
            }
        })
    }
}

#[derive(Default)]
pub(super) struct FakePredicates {
    pub(super) reasoning: Vec<&'static str>,
    pub(super) asked: Vec<String>,
}

impl CapabilityPredicates for FakePredicates {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        self.asked.push(format!("{key}:{}", request.model));
        ready(key == "supports_reasoning" && self.reasoning.contains(&request.model))
    }

    fn declared_value_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn supports_native_streaming(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = Json> + Send {
        ready(Json::Null)
    }

    fn supports_response_schema(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }
}

pub(super) fn config(
    credentials: FakeCredentials,
    predicates: FakePredicates,
    settings: Settings,
) -> GithubCopilotConfig<FakeCredentials, FakePredicates> {
    GithubCopilotConfig::new(
        None,
        Headers::new(),
        credentials,
        predicates,
        settings,
        CatalogCapabilities {
            chat_models: BTreeSet::new(),
            text_models: BTreeSet::new(),
        },
    )
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}
