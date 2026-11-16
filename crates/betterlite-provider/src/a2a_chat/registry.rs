use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::SecretString;

use super::auth::{EntraResolver, a2a_hop_uses_entra};
use super::error::A2aFailure;
use super::request::stream_requested;

// LiteLLM keeps the transport fields and every Entra credential key out of optional_params
pub(crate) const REGISTRY_PARAMS_KEPT_OUT: [&str; 11] = [
    "api_base",
    "api_key",
    "azure_ad_token",
    "azure_password",
    "azure_scope",
    "azure_username",
    "client_id",
    "client_secret",
    "headers",
    "model",
    "tenant_id",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredA2aAgent {
    pub agent_card_params: Object,
    pub litellm_params: Object,
}

pub trait A2aAgentLookup {
    fn agent_by_name(&self, name: &str) -> Option<RegisteredA2aAgent>;
}

pub struct RegistryInput<'request> {
    pub agent_name: &'request str,
    pub api_base: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
    pub headers: Option<&'request BTreeMap<String, String>>,
}

#[derive(Debug, Clone)]
pub struct RegistryResolved {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub headers: Option<BTreeMap<String, String>>,
}

fn unchanged(input: &RegistryInput<'_>) -> RegistryResolved {
    RegistryResolved {
        api_base: input.api_base.map(str::to_owned),
        api_key: input.api_key.cloned(),
        headers: input.headers.cloned(),
    }
}

fn card_url(card: &Object) -> Result<Option<String>, A2aFailure> {
    match card.get("url") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(url)) => Ok(Some(url.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(A2aFailure::CardUrlType)
        }
    }
}

fn custom_provider(params: &Object) -> Result<Option<&str>, A2aFailure> {
    match params.get("custom_llm_provider") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(provider)) => Ok(Some(provider)),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(A2aFailure::OptionType {
                field: "custom_llm_provider",
            })
        }
    }
}

fn registry_api_key(
    params: &Object,
    uses_entra: bool,
    entra: &dyn EntraResolver,
) -> Result<Option<SecretString>, A2aFailure> {
    if uses_entra {
        let token = entra.token(params)?;
        return Ok(Some(token));
    }
    match params.get("api_key") {
        Some(Json::Text(key)) => Ok(Some(SecretString::from(key.clone()))),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Ok(None)
        }
    }
}

fn registry_headers(
    params: &Object,
    uses_entra: bool,
) -> Result<Option<BTreeMap<String, String>>, A2aFailure> {
    let Some(Json::Object(stored)) = params.get("headers") else {
        return Ok(None);
    };
    let mut headers = BTreeMap::new();
    for (name, value) in stored {
        if uses_entra && name.eq_ignore_ascii_case("authorization") {
            continue;
        }
        let Json::Text(text) = value else {
            return Err(A2aFailure::HeaderType { name: name.clone() });
        };
        headers.insert(name.clone(), text.clone());
    }
    Ok(Some(headers))
}

fn card_declares_no_streaming(card: &Object) -> bool {
    let Some(Json::Object(capabilities)) = card.get("capabilities") else {
        return false;
    };
    !capabilities.get("streaming").is_some_and(Json::is_truthy)
}

fn merge_params(
    agent: &RegisteredA2aAgent,
    optional_params: &mut Object,
) -> Result<(), A2aFailure> {
    let stream = stream_requested(optional_params)?;
    for (key, value) in &agent.litellm_params {
        if REGISTRY_PARAMS_KEPT_OUT.contains(&key.as_str()) || optional_params.contains_key(key) {
            continue;
        }
        optional_params.insert(key.clone(), value.clone());
    }
    if stream && card_declares_no_streaming(&agent.agent_card_params) {
        optional_params.insert(String::from("stream"), Json::Bool(false));
        optional_params.insert(String::from("fake_stream"), Json::Bool(true));
    }
    Ok(())
}

fn fill_from_agent(
    input: &RegistryInput<'_>,
    agent: &RegisteredA2aAgent,
    optional_params: &mut Object,
    entra: &dyn EntraResolver,
) -> Result<RegistryResolved, A2aFailure> {
    let uses_entra = a2a_hop_uses_entra(
        Some(&agent.litellm_params),
        custom_provider(&agent.litellm_params)?,
    );
    let mut api_base = input.api_base.map(str::to_owned);
    if api_base.is_none() && !agent.agent_card_params.is_empty() {
        api_base = card_url(&agent.agent_card_params)?;
    }
    let mut api_key = input.api_key.cloned();
    if api_key.is_none() {
        api_key = registry_api_key(&agent.litellm_params, uses_entra, entra)?;
    }
    let mut headers = input.headers.cloned();
    if !input.headers.is_some_and(|given| !given.is_empty()) {
        headers = registry_headers(&agent.litellm_params, uses_entra)?
            .filter(|stored| !stored.is_empty())
            .or(headers);
    }
    merge_params(agent, optional_params)?;
    Ok(RegistryResolved {
        api_base,
        api_key,
        headers,
    })
}

pub fn resolve_agent_config(
    input: &RegistryInput<'_>,
    optional_params: &mut Object,
    lookup: Option<&dyn A2aAgentLookup>,
    entra: &dyn EntraResolver,
) -> Result<RegistryResolved, A2aFailure> {
    let explicit = input.api_base.is_some()
        && input.api_key.is_some()
        && input.headers.is_some_and(|headers| !headers.is_empty());
    if input.agent_name.is_empty() || explicit {
        return Ok(unchanged(input));
    }
    let Some(lookup) = lookup else {
        return Ok(unchanged(input));
    };
    let Some(agent) = lookup.agent_by_name(input.agent_name) else {
        return Ok(unchanged(input));
    };
    fill_from_agent(input, &agent, optional_params, entra)
}
