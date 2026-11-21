use betterlite_config::sections::complexity_turn_overrides::KeywordMatching;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatMessage;

use crate::classifier_verdict::call::RequestKwargs;
use crate::complexity_scoring::tier::Tier;
use crate::semantic_router::service::{CallerContext, RouteEmbedder};
use crate::tier_definitions::model::TierSet;

use super::error::TurnOverrideError;
use super::housekeeping::{cheapest_configured_tier, matched_housekeeping_sentinel};
use super::keywords::{KeywordOverride, LexicalRules, matched_escalation_keyword};
use super::plan::{matched_plan_mode_signal, plan_mode_floor, plan_mode_floor_is_top_tier};
use super::semantic::SemanticKeywordIndex;
use super::settings::Settings;
use super::stall::detect_stalled_task;

// everything the overrides read from one request; B335 supplies the newest ask and the caller identity
pub struct TurnRequest<'request> {
    pub request_kwargs: Option<&'request RequestKwargs>,
    pub resolved_messages: Option<&'request [ChatMessage]>,
    pub messages: Option<&'request [Json]>,
    pub user_message: &'request str,
    pub newest_ask: Option<&'request str>,
    pub caller: &'request CallerContext,
}

// the per-turn observations B335 applies around the classifier verdict; none of them touches a session pin
#[derive(Debug, Clone, PartialEq)]
pub struct TurnOverrides {
    pub plan_mode_sentinel: Option<String>,
    pub plan_mode_floor: Option<Tier>,
    pub plan_mode_floor_is_top_tier: bool,
    pub escalation_keyword: Option<String>,
    pub stalled: bool,
    pub housekeeping_sentinel: Option<String>,
    pub housekeeping_tier: Option<Tier>,
    pub keyword_override: Option<KeywordOverride>,
}

pub async fn resolve_turn<E>(
    settings: &Settings,
    catalog: &TierSet,
    lexical: &LexicalRules,
    semantic: Option<&SemanticKeywordIndex<E>>,
    request: &TurnRequest<'_>,
) -> Result<TurnOverrides, TurnOverrideError>
where
    E: RouteEmbedder + 'static,
{
    let keyword_override = resolve_keyword_override(settings, lexical, semantic, request).await?;
    let plan_mode_sentinel =
        matched_plan_mode_signal(settings, request.request_kwargs, request.resolved_messages)?;
    let floor = plan_mode_floor(settings, catalog)?;
    let floor_is_top = plan_mode_floor_is_top_tier(settings, catalog)?;
    let escalation_keyword = matched_escalation_keyword(settings, catalog, request.user_message);
    let stalled = stalled_task(settings, request.messages);
    let (housekeeping_sentinel, housekeeping_tier) =
        housekeeping(settings, catalog, request.newest_ask)?;
    Ok(TurnOverrides {
        plan_mode_sentinel,
        plan_mode_floor: floor,
        plan_mode_floor_is_top_tier: floor_is_top,
        escalation_keyword,
        stalled,
        housekeeping_sentinel,
        housekeeping_tier,
        keyword_override,
    })
}

fn stalled_task(settings: &Settings, messages: Option<&[Json]>) -> bool {
    if !settings.stall_escalation_enabled {
        return false;
    }
    detect_stalled_task(
        messages,
        settings.stall_escalation_window,
        settings.stall_repeat_threshold,
    )
}

// a housekeeping ask routes to the cheapest configured tier that has models, with no classifier spend
fn housekeeping(
    settings: &Settings,
    catalog: &TierSet,
    newest_ask: Option<&str>,
) -> Result<(Option<String>, Option<Tier>), TurnOverrideError> {
    let sentinel = matched_housekeeping_sentinel(settings, newest_ask);
    let tier = if sentinel.is_some() {
        cheapest_configured_tier(catalog)?
    } else {
        None
    };
    Ok((sentinel, tier))
}

// _resolve_keyword_tier_override: rules first, then the configured matcher; a semantic failure is no override
async fn resolve_keyword_override<E>(
    settings: &Settings,
    lexical: &LexicalRules,
    semantic: Option<&SemanticKeywordIndex<E>>,
    request: &TurnRequest<'_>,
) -> Result<Option<KeywordOverride>, TurnOverrideError>
where
    E: RouteEmbedder + 'static,
{
    if settings.keyword_tier_rules.is_empty() {
        return Ok(None);
    }
    if settings.keyword_matching == KeywordMatching::Lexical {
        return Ok(lexical.matches(request.user_message));
    }
    let Some(index) = semantic else {
        return Ok(None);
    };
    let tier = index
        .tier_override(request.user_message, request.caller)
        .await;
    Ok(tier.map(|tier| KeywordOverride {
        tier,
        matched_keyword: None,
    }))
}
