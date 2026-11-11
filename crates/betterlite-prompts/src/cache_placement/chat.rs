use std::collections::BTreeMap;

use crate::arguments::json::{Json, Object};
use crate::cache_accounting::census::{points_with_a_slot_left, reserved_outside_messages};
use crate::cache_accounting::constants::{
    CARRY_UNMATCHED_MESSAGE_POINTS, MAX_CACHE_CONTROL_BLOCKS,
};
use crate::cache_accounting::dialect::{DialectInputs, ModelFacts, targets_openai_breakpoint};
use crate::cache_accounting::points::{CacheControlInjectionPoint, MessageInjectionPoint};
use crate::cache_placement::error::PlacementError;
use crate::cache_placement::marks::count_request_marks;
use crate::cache_placement::message_injections::apply_message_injections;
use crate::cache_placement::points::{first_dialect, first_external, message_point};
use crate::message_content::Message;

const INJECTION_POINTS: &str = "cache_control_injection_points";
const PROMPT_CACHE_OPTIONS: &str = "prompt_cache_options";

pub struct ChatPrompt<'input> {
    pub model: &'input str,
    pub messages: &'input [Message],
    pub params: &'input mut Object,
    pub environment: &'input BTreeMap<String, String>,
    pub global_api_base: Option<&'input str>,
    pub facts: &'input dyn ModelFacts,
}

struct DialectSource<'input> {
    model: &'input str,
    params: &'input Object,
    environment: &'input BTreeMap<String, String>,
    global_api_base: Option<&'input str>,
    facts: &'input dyn ModelFacts,
}

struct PointPlan<'input> {
    message: Vec<&'input MessageInjectionPoint>,
    remaining: Vec<CacheControlInjectionPoint>,
    max_blocks: usize,
    external: usize,
}

pub fn apply_chat_injections(prompt: ChatPrompt<'_>) -> Result<Vec<Message>, PlacementError> {
    let ChatPrompt {
        model,
        messages,
        params,
        environment,
        global_api_base,
        facts,
    } = prompt;
    let carry_unmatched = matches!(
        params.shift_remove(CARRY_UNMATCHED_MESSAGE_POINTS),
        Some(Json::Bool(true))
    );
    let Some(points) = points_from(params)? else {
        return Ok(messages.to_vec());
    };
    let dialect = first_dialect(&points).unwrap_or_else(|| {
        let source = DialectSource {
            model,
            params,
            environment,
            global_api_base,
            facts,
        };
        fallback_dialect(&source)
    });
    let plan = point_plan(&points, dialect);
    let (applied, carried) = split_message_points(&plan.message, carry_unmatched);
    let before = count_request_marks(messages, None);
    let processed = apply_message_injections(&applied, messages, plan.max_blocks, dialect);
    let after = count_request_marks(&processed, None);
    if dialect && after > before {
        set_explicit_mode(params);
    }
    let on_wire = after.saturating_add(plan.external);
    carry_forward(params, &plan.remaining, &carried, on_wire, dialect)?;
    Ok(processed)
}

fn points_from(
    params: &mut Object,
) -> Result<Option<Vec<CacheControlInjectionPoint>>, PlacementError> {
    let Some(raw) = params.shift_remove(INJECTION_POINTS) else {
        return Ok(None);
    };
    if matches!(raw, Json::Null) {
        return Ok(None);
    }
    let value =
        serde_json::to_value(&raw).map_err(|source| PlacementError::InjectionPoints { source })?;
    let points: Vec<CacheControlInjectionPoint> = serde_json::from_value(value)
        .map_err(|source| PlacementError::InjectionPoints { source })?;
    Ok((!points.is_empty()).then_some(points))
}

fn fallback_dialect(source: &DialectSource<'_>) -> bool {
    let inputs = DialectInputs {
        model: source.model,
        custom_llm_provider: source
            .params
            .get("custom_llm_provider")
            .and_then(Json::as_str),
        api_base: source.params.get("api_base").and_then(Json::as_str),
        base_url: source.params.get("base_url").and_then(Json::as_str),
        global_api_base: source.global_api_base,
        environment: source.environment,
        prompt_cache_options: source.params.get(PROMPT_CACHE_OPTIONS),
    };
    targets_openai_breakpoint(&inputs, source.facts)
}

fn point_plan(points: &[CacheControlInjectionPoint], dialect: bool) -> PointPlan<'_> {
    let message = points.iter().filter_map(message_point).collect();
    let remaining: Vec<CacheControlInjectionPoint> = points
        .iter()
        .filter(|point| message_point(point).is_none())
        .cloned()
        .collect();
    let external = first_external(points).unwrap_or(0);
    let reserved = reserved_outside_messages(&remaining, external, dialect);
    PointPlan {
        message,
        remaining,
        max_blocks: MAX_CACHE_CONTROL_BLOCKS.saturating_sub(reserved),
        external,
    }
}

fn split_message_points<'points>(
    message_points: &[&'points MessageInjectionPoint],
    carry_unmatched: bool,
) -> (
    Vec<&'points MessageInjectionPoint>,
    Vec<&'points MessageInjectionPoint>,
) {
    if carry_unmatched {
        message_points
            .iter()
            .copied()
            .partition(|point| point.index.is_some())
    } else {
        (message_points.to_vec(), Vec::new())
    }
}

fn set_explicit_mode(params: &mut Object) {
    let mut options = Object::new();
    options.insert("mode".to_owned(), Json::Text("explicit".to_owned()));
    params
        .entry(PROMPT_CACHE_OPTIONS.to_owned())
        .or_insert(Json::Object(options));
}

fn carry_forward(
    params: &mut Object,
    remaining: &[CacheControlInjectionPoint],
    carried: &[&MessageInjectionPoint],
    on_wire: usize,
    dialect: bool,
) -> Result<(), PlacementError> {
    let mut forwarded = points_with_a_slot_left(remaining, on_wire, dialect);
    forwarded.extend(
        carried
            .iter()
            .map(|point| CacheControlInjectionPoint::Message((*point).clone())),
    );
    if forwarded.is_empty() {
        return Ok(());
    }
    let value = serde_json::to_value(&forwarded)
        .map_err(|source| PlacementError::PointEncoding { source })?;
    let encoded: Json =
        serde_json::from_value(value).map_err(|source| PlacementError::PointEncoding { source })?;
    params.insert(INJECTION_POINTS.to_owned(), encoded);
    Ok(())
}
