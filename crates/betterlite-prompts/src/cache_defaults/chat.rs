use std::collections::BTreeMap;

use crate::arguments::json::{Json, Object};
use crate::cache_accounting::carriers::drops_tool_cache_control;
use crate::cache_accounting::census::count_external_breakpoints;
use crate::cache_accounting::constants::MAX_CACHE_CONTROL_BLOCKS;
use crate::cache_accounting::dialect::DialectInputs;
use crate::cache_accounting::points::CacheControlInjectionPoint;
use crate::cache_accounting::stamping::stamped_for_prompt_hook;
use crate::cache_placement::message_injections::apply_message_injections;
use crate::cache_placement::points::message_point;
use crate::cache_placement::system::SystemPrompt;
use crate::message_content::Message;

use super::selection::{DefaultRequest, get_default_injection_points, request_view};
use super::{CacheFacts, CachePolicy, DefaultsError, INJECTION_POINTS, encode_points};

pub struct RouterProjection<'input> {
    pub messages: &'input [Message],
    pub models: &'input [String],
    pub tools: Option<&'input [Json]>,
    pub enable_prompt_caching: Option<bool>,
    pub request_kwargs: &'input Object,
    pub alias_map: &'input BTreeMap<String, String>,
    pub policy: CachePolicy<'input>,
}

// Router cache affinity reproduces the messages the provider will see, so the cache key matches the success event.
#[must_use]
pub fn messages_with_defaults(
    projection: &RouterProjection<'_>,
    facts: &dyn CacheFacts,
) -> Vec<Message> {
    let system = typed_system(projection.request_kwargs);
    let cache_control = projection.request_kwargs.get("cache_control");
    for model in projection.models {
        let resolved = projection
            .alias_map
            .get(model)
            .map_or(model.as_str(), String::as_str);
        let request = DefaultRequest {
            model: resolved,
            custom_llm_provider: None,
            messages: projection.messages,
            system: system.as_ref(),
            tools: projection.tools,
            enable_prompt_caching: projection.enable_prompt_caching,
            cache_control,
            request_kwargs: projection.request_kwargs,
            on_messages_route: false,
            policy: projection.policy,
        };
        let points = get_default_injection_points(&request, facts);
        if !points.is_empty() {
            let message_points: Vec<_> = points.iter().filter_map(message_point).collect();
            return apply_message_injections(
                &message_points,
                projection.messages,
                MAX_CACHE_CONTROL_BLOCKS,
                false,
            );
        }
    }
    projection.messages.to_vec()
}

fn typed_system(request_kwargs: &Object) -> Option<SystemPrompt> {
    match request_kwargs.get("system")? {
        Json::Text(text) => Some(SystemPrompt::Text(text.clone())),
        Json::List(items) => {
            let mut blocks = Vec::with_capacity(items.len());
            for item in items {
                let Json::Object(fields) = item else {
                    return None;
                };
                blocks.push(fields.clone());
            }
            Some(SystemPrompt::Blocks(blocks))
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => None,
    }
}

pub struct ChatSeeding<'input> {
    pub model: &'input str,
    pub custom_llm_provider: Option<&'input str>,
    pub messages: &'input [Message],
    pub params: &'input mut Object,
    pub tools: Option<&'input [Json]>,
    pub enable_prompt_caching: Option<bool>,
    pub api_base: Option<&'input str>,
    pub global_api_base: Option<&'input str>,
    pub environment: &'input BTreeMap<String, String>,
    pub alias_map: &'input BTreeMap<String, String>,
    pub policy: CachePolicy<'input>,
}

// Configured points win over defaults; the seeded list is what the prompt gate and placement pass will consume.
pub fn seed_default_injection_points(
    mut seeding: ChatSeeding<'_>,
    facts: &dyn CacheFacts,
) -> Result<(), DefaultsError> {
    let Some(configured) = configured_points(seeding.params)? else {
        return seed_defaults(&mut seeding, facts);
    };
    stamp_configured(&mut seeding, &configured, facts)
}

fn stamp_configured(
    seeding: &mut ChatSeeding<'_>,
    configured: &[CacheControlInjectionPoint],
    facts: &dyn CacheFacts,
) -> Result<(), DefaultsError> {
    let kept_tools: Vec<Json> = seeding
        .tools
        .unwrap_or_default()
        .iter()
        .filter(|tool| !drops_tool_cache_control(tool))
        .cloned()
        .collect();
    let external = {
        let wire = request_view(seeding.params);
        count_external_breakpoints(
            Some(&kept_tools),
            seeding.params.get("cache_control"),
            Some(&wire),
        )
    };
    let inputs = DialectInputs {
        model: seeding.model,
        custom_llm_provider: seeding.custom_llm_provider,
        api_base: seeding.api_base,
        base_url: None,
        global_api_base: seeding.global_api_base,
        environment: seeding.environment,
        prompt_cache_options: seeding.params.get("prompt_cache_options"),
    };
    let stamped = stamped_for_prompt_hook(configured, external, &inputs, facts);
    let encoded = encode_points(&stamped)?;
    seeding.params.insert(INJECTION_POINTS.to_owned(), encoded);
    Ok(())
}

fn seed_defaults(
    seeding: &mut ChatSeeding<'_>,
    facts: &dyn CacheFacts,
) -> Result<(), DefaultsError> {
    let resolved = seeding
        .alias_map
        .get(seeding.model)
        .map_or(seeding.model, String::as_str);
    let request = DefaultRequest {
        model: resolved,
        custom_llm_provider: seeding.custom_llm_provider,
        messages: seeding.messages,
        system: None,
        tools: seeding.tools,
        enable_prompt_caching: seeding.enable_prompt_caching,
        cache_control: seeding.params.get("cache_control"),
        request_kwargs: seeding.params,
        on_messages_route: false,
        policy: seeding.policy,
    };
    let points = get_default_injection_points(&request, facts);
    if !points.is_empty() {
        let encoded = encode_points(&points)?;
        seeding.params.insert(INJECTION_POINTS.to_owned(), encoded);
    }
    Ok(())
}

fn configured_points(
    params: &Object,
) -> Result<Option<Vec<CacheControlInjectionPoint>>, DefaultsError> {
    let Some(value) = params.get(INJECTION_POINTS) else {
        return Ok(None);
    };
    if matches!(value, Json::Null) || matches!(&value, Json::List(items) if items.is_empty()) {
        return Ok(None);
    }
    let encoded =
        serde_json::to_value(value).map_err(|source| DefaultsError::InjectionPoints { source })?;
    let points = serde_json::from_value(encoded)
        .map_err(|source| DefaultsError::InjectionPoints { source })?;
    Ok(Some(points))
}
