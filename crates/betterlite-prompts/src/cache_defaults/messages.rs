use std::collections::BTreeMap;

use crate::arguments::json::{Json, Object};
use crate::cache_accounting::census::count_route_breakpoints;
use crate::cache_accounting::dialect::{DialectInputs, targets_openai_breakpoint};
use crate::cache_accounting::points::CacheControlInjectionPoint;
use crate::cache_placement::marks::count_request_marks;
use crate::cache_placement::messages_route::apply_messages_route;
use crate::cache_placement::system::SystemPrompt;
use crate::message_content::Message;

use super::attribution::record_gateway_injection;
use super::selection::{DefaultRequest, get_default_injection_points, request_view};
use super::{CacheFacts, CachePolicy, DefaultsError, INJECTION_POINTS, encode_points};

const ENABLE_PROMPT_CACHING: &str = "enable_prompt_caching";
const PROMPT_CACHE_OPTIONS: &str = "prompt_cache_options";
const EXPLICIT_MODE: &str = "mode";
const EXPLICIT: &str = "explicit";

pub struct MessagesInjection<'input> {
    pub messages: &'input [Message],
    pub system: Option<&'input SystemPrompt>,
    pub params: &'input mut Object,
    pub model: Option<&'input str>,
    pub custom_llm_provider: Option<&'input str>,
    pub tools: Option<&'input [Json]>,
    pub api_base: Option<&'input str>,
    pub global_api_base: Option<&'input str>,
    pub environment: &'input BTreeMap<String, String>,
    pub policy: CachePolicy<'input>,
}

pub struct PlacedMessages {
    pub messages: Vec<Message>,
    pub system: Option<SystemPrompt>,
}

// The native Messages route applies points to the payload it sends and hands the rest to downstream transforms.
pub fn maybe_inject_cache_control(
    mut injection: MessagesInjection<'_>,
    facts: &dyn CacheFacts,
) -> Result<PlacedMessages, DefaultsError> {
    let enable_prompt_caching = take_enable_prompt_caching(injection.params)?;
    let cache_control = injection.params.get("cache_control").cloned();
    let configured = take_configured_points(injection.params)?;
    let points = configured.unwrap_or_else(|| {
        default_points(
            &injection,
            enable_prompt_caching,
            cache_control.as_ref(),
            facts,
        )
    });
    if points.is_empty() {
        return Ok(PlacedMessages {
            messages: injection.messages.to_vec(),
            system: injection.system.cloned(),
        });
    }
    place_messages(&mut injection, &points, cache_control.as_ref(), facts)
}

fn default_points(
    injection: &MessagesInjection<'_>,
    enable_prompt_caching: Option<bool>,
    cache_control: Option<&Json>,
    facts: &dyn CacheFacts,
) -> Vec<CacheControlInjectionPoint> {
    let Some(model) = injection.model else {
        return Vec::new();
    };
    get_default_injection_points(
        &DefaultRequest {
            model,
            custom_llm_provider: injection.custom_llm_provider,
            messages: injection.messages,
            system: injection.system,
            tools: injection.tools,
            enable_prompt_caching,
            cache_control,
            request_kwargs: injection.params,
            on_messages_route: true,
            policy: injection.policy,
        },
        facts,
    )
}

fn place_messages(
    injection: &mut MessagesInjection<'_>,
    points: &[CacheControlInjectionPoint],
    cache_control: Option<&Json>,
    facts: &dyn CacheFacts,
) -> Result<PlacedMessages, DefaultsError> {
    let openai_dialect = openai_dialect_for(injection, facts);
    let before = count_request_marks(injection.messages, injection.system);
    let wire = request_view(injection.params);
    let external = count_route_breakpoints(injection.tools, cache_control, Some(&wire));
    let placed = apply_messages_route(
        injection.messages,
        injection.system,
        points,
        openai_dialect,
        external,
    );
    let after = count_request_marks(&placed.messages, placed.system.as_ref());
    let added = i32::try_from(after.saturating_sub(before)).unwrap_or(i32::MAX);
    record_gateway_injection(injection.params, added, false);
    if openai_dialect && added > 0_i32 {
        set_explicit_mode(injection.params);
    }
    write_remaining(injection.params, &placed.remaining)?;
    Ok(PlacedMessages {
        messages: placed.messages,
        system: placed.system,
    })
}

fn openai_dialect_for(injection: &MessagesInjection<'_>, facts: &dyn CacheFacts) -> bool {
    injection.model.is_some_and(|model| {
        targets_openai_breakpoint(
            &DialectInputs {
                model,
                custom_llm_provider: injection.custom_llm_provider,
                api_base: injection.api_base,
                base_url: None,
                global_api_base: injection.global_api_base,
                environment: injection.environment,
                prompt_cache_options: injection.params.get(PROMPT_CACHE_OPTIONS),
            },
            facts,
        )
    })
}

fn write_remaining(
    params: &mut Object,
    remaining: &[CacheControlInjectionPoint],
) -> Result<(), DefaultsError> {
    if remaining.is_empty() {
        return Ok(());
    }
    let encoded = encode_points(remaining)?;
    params.insert(INJECTION_POINTS.to_owned(), encoded);
    Ok(())
}

fn take_enable_prompt_caching(params: &mut Object) -> Result<Option<bool>, DefaultsError> {
    match params.shift_remove(ENABLE_PROMPT_CACHING) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(flag)) => Ok(Some(flag)),
        Some(Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(DefaultsError::EnablePromptCaching)
        }
    }
}

fn take_configured_points(
    params: &mut Object,
) -> Result<Option<Vec<CacheControlInjectionPoint>>, DefaultsError> {
    let Some(value) = params.shift_remove(INJECTION_POINTS) else {
        return Ok(None);
    };
    if matches!(value, Json::Null) || matches!(&value, Json::List(items) if items.is_empty()) {
        return Ok(None);
    }
    let encoded =
        serde_json::to_value(&value).map_err(|source| DefaultsError::InjectionPoints { source })?;
    let points = serde_json::from_value(encoded)
        .map_err(|source| DefaultsError::InjectionPoints { source })?;
    Ok(Some(points))
}

fn set_explicit_mode(params: &mut Object) {
    let mut options = Object::new();
    options.insert(EXPLICIT_MODE.to_owned(), Json::Text(EXPLICIT.to_owned()));
    params
        .entry(PROMPT_CACHE_OPTIONS.to_owned())
        .or_insert(Json::Object(options));
}
