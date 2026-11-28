mod credential;
mod known_values;
mod litellm_settings;
mod render;
mod sections;
mod tables;
mod tests;

use std::collections::BTreeMap;
use std::panic::Location;

use serde::Deserialize;

use crate::bug_report::environment::{Host, build_environment_report};
use crate::bug_report::record::{BugReport, EnvironmentReport, Surface};
use crate::bug_report::report::{CallContext, build_bug_report};
use crate::safe_proxy_config::litellm_settings::LITELLM_SETTINGS_KEYS;
use crate::safe_proxy_config::sections::{
    guardrail_lines, model_list_lines, object_at, section_lines,
};
use crate::safe_proxy_config::tables::{
    CACHE_PARAMS_KEYS, GENERAL_SETTINGS_KEYS, ROUTER_SETTINGS_KEYS,
};

pub type ConfigObject = BTreeMap<String, ConfigValue>;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum ConfigValue {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    Text(String),
    List(Vec<Self>),
    Object(ConfigObject),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ProxyConfigSnapshot {
    pub config: ConfigObject,
    pub general_settings: ConfigObject,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProxyCall {
    pub call_type: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub stream: Option<bool>,
}

#[must_use]
pub fn safe_config_lines(config: &ConfigObject, general_settings: &ConfigObject) -> Vec<String> {
    let mut lines = section_lines(
        "general_settings",
        Some(general_settings),
        GENERAL_SETTINGS_KEYS,
    );
    let settings = object_at(config, "litellm_settings");
    lines.extend(section_lines(
        "litellm_settings",
        settings,
        LITELLM_SETTINGS_KEYS,
    ));
    let cache = settings.and_then(|params| object_at(params, "cache_params"));
    lines.extend(section_lines(
        "litellm_settings.cache_params",
        cache,
        CACHE_PARAMS_KEYS,
    ));
    let router = object_at(config, "router_settings");
    lines.extend(section_lines(
        "router_settings",
        router,
        ROUTER_SETTINGS_KEYS,
    ));
    lines.extend(guardrail_lines(config.get("guardrails")));
    lines.extend(model_list_lines(config.get("model_list")));
    lines
}

#[must_use]
pub fn build_proxy_environment_report(
    snapshot: &ProxyConfigSnapshot,
    host: Host<'_>,
) -> EnvironmentReport {
    build_environment_report(
        Surface::Proxy,
        host,
        safe_config_lines(&snapshot.config, &snapshot.general_settings),
    )
}

#[must_use]
pub fn build_proxy_bug_report(
    exception_type: &str,
    location: &Location<'_>,
    call: ProxyCall,
    snapshot: &ProxyConfigSnapshot,
    host: Host<'_>,
) -> BugReport {
    build_bug_report(
        exception_type,
        location,
        CallContext {
            surface: Surface::Proxy,
            call_type: call.call_type,
            custom_llm_provider: call.custom_llm_provider,
            stream: call.stream,
            config_lines: safe_config_lines(&snapshot.config, &snapshot.general_settings),
        },
        host,
    )
}
