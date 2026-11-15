#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use betterlite_config::sections::openai_credentials::Settings;
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde_json::{Map, Value};

use crate::api::container_contracts::config::ContainerRequestContext;
use crate::containers::OpenAiContainerConfig;
use crate::containers::costs::ContainerCosts;
use crate::credentials::LiteLlmGlobals;

pub(super) const BASE: &str = "https://api.openai.com/v1/containers";
pub(super) const AZURE: &str =
    "https://example.openai.azure.com/openai/v1/containers?api-version=v1";

pub(super) const CONTAINER_BODY: &str = r#"{"id":"cntr_abc","object":"container","created_at":1700000000,"status":"running","expires_after":{"anchor":"last_active_at","minutes":20},"last_active_at":null,"name":"box"}"#;
pub(super) const CONTAINER_NULL_BODY: &str = r#"{"id":"cntr_none","object":"container","created_at":1700000001,"status":"running","expires_after":null,"last_active_at":null,"name":null,"unknown_extra":{"a":1}}"#;
pub(super) const CONTAINER_WRONG_OBJECT: &str =
    r#"{"id":"cntr_bad","object":"container.deleted","created_at":1700000002,"status":"running"}"#;
pub(super) const LIST_BODY: &str = r#"{"object":"list","data":[{"id":"cntr_a","object":"container","created_at":1,"status":"running","expires_after":null,"last_active_at":1730000000,"name":null},{"id":"cntr_b","object":"container","created_at":2,"status":"queued","expires_after":{"anchor":"last_active_at","minutes":5},"last_active_at":null,"name":"b"}],"first_id":"cntr_a","last_id":"cntr_b","has_more":true}"#;
pub(super) const FILE_LIST_BODY: &str = r#"{"object":"list","data":[{"id":"cfile_1","object":"container.file","container_id":"cntr_abc","bytes":12,"created_at":3,"path":"/mnt/data/a.txt","source":"user"}],"first_id":"cfile_1","last_id":null,"has_more":false}"#;
pub(super) const DELETE_BODY: &str =
    r#"{"id":"cntr_abc","object":"container.deleted","deleted":true}"#;

#[derive(Debug)]
pub(super) struct ProviderCost;

impl ContainerCosts for ProviderCost {
    fn code_interpreter_cost(&self, sessions: u32, provider: &str) -> Decimal {
        if sessions == 1 && provider == "openai" {
            Decimal::new(3, 2)
        } else {
            Decimal::ZERO
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct RecorderCost {
    calls: Mutex<Vec<(u32, String)>>,
}

impl ContainerCosts for RecorderCost {
    fn code_interpreter_cost(&self, sessions: u32, provider: &str) -> Decimal {
        self.calls
            .lock()
            .unwrap()
            .push((sessions, provider.to_owned()));
        Decimal::new(3, 2)
    }
}

impl RecorderCost {
    pub(super) fn calls(&self) -> Vec<(u32, String)> {
        self.calls.lock().unwrap().clone()
    }
}

pub(super) fn settings_with(api_base: &str) -> Settings {
    Settings {
        api_base: api_base.to_owned(),
        organization: None,
    }
}

pub(super) fn globals_with(api_key: Option<&str>, openai_key: Option<&str>) -> LiteLlmGlobals {
    LiteLlmGlobals {
        api_base: None,
        api_key: api_key.map(SecretString::from),
        openai_key: openai_key.map(SecretString::from),
        organization: None,
    }
}

pub(super) fn globals_with_base(api_base: &str) -> LiteLlmGlobals {
    LiteLlmGlobals {
        api_base: Some(api_base.to_owned()),
        api_key: None,
        openai_key: None,
        organization: None,
    }
}

pub(super) fn config_with(
    settings: Settings,
    globals: LiteLlmGlobals,
    env_api_key: Option<&str>,
) -> OpenAiContainerConfig {
    OpenAiContainerConfig {
        settings,
        globals,
        env_api_key: env_api_key.map(SecretString::from),
        costs: Arc::new(ProviderCost),
    }
}

pub(super) fn config_with_costs(costs: Arc<dyn ContainerCosts>) -> OpenAiContainerConfig {
    OpenAiContainerConfig {
        settings: Settings::default(),
        globals: LiteLlmGlobals::default(),
        env_api_key: None,
        costs,
    }
}

pub(super) fn fresh_config() -> OpenAiContainerConfig {
    config_with(Settings::default(), LiteLlmGlobals::default(), None)
}

pub(super) struct Context {
    litellm_params: Map<String, Value>,
    headers: BTreeMap<String, String>,
}

impl Context {
    pub(super) fn empty() -> Self {
        Self {
            litellm_params: Map::new(),
            headers: BTreeMap::new(),
        }
    }

    pub(super) fn request(&self) -> ContainerRequestContext<'_> {
        ContainerRequestContext {
            litellm_params: &self.litellm_params,
            headers: &self.headers,
        }
    }
}
