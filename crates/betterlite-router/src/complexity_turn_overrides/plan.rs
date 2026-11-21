use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatMessage;

use crate::classifier_message_text::error::ClassifierTextError;
use crate::classifier_message_text::markers::{
    MarkerPair, is_codex_user_agent, reminder_markers_for_request,
};
use crate::classifier_message_text::plan::matched_plan_mode_sentinel;
use crate::classifier_verdict::call::RequestKwargs;
use crate::complexity_scoring::tier::Tier;
use crate::tier_definitions::model::TierSet;

use super::error::TurnOverrideError;
use super::settings::{Settings, active_tier, tier_severity};

// _resolve_plan_mode_floor: the configured floor as an active tier, or None when the feature is off
pub fn plan_mode_floor(
    settings: &Settings,
    catalog: &TierSet,
) -> Result<Option<Tier>, TurnOverrideError> {
    match settings.plan_mode_min_tier.as_deref() {
        Some(name) => active_tier(catalog, name).map(Some),
        None => Ok(None),
    }
}

// _apply_plan_mode_floor: the higher of the decided tier and the floor, identity when the floor is unset
pub fn apply_plan_mode_floor(
    settings: &Settings,
    catalog: &TierSet,
    tier: Tier,
) -> Result<Tier, TurnOverrideError> {
    let Some(floor) = plan_mode_floor(settings, catalog)? else {
        return Ok(tier);
    };
    let tier_rank = tier_severity(catalog, &tier)?;
    let floor_rank = tier_severity(catalog, &floor)?;
    Ok(if tier_rank >= floor_rank { tier } else { floor })
}

// _plan_mode_floor_is_top_tier: no configured tier outranks the floor, so classification could never rise above it
pub fn plan_mode_floor_is_top_tier(
    settings: &Settings,
    catalog: &TierSet,
) -> Result<bool, TurnOverrideError> {
    let Some(floor) = plan_mode_floor(settings, catalog)? else {
        return Ok(false);
    };
    let floor_rank = tier_severity(catalog, &floor)?;
    Ok(catalog
        .tier_names()
        .iter()
        .skip(floor_rank.saturating_add(1))
        .all(|name| !catalog.pools.contains_key(*name)))
}

// _matched_plan_mode_signal: the sentinel on this request, always None while the floor is unset
pub fn matched_plan_mode_signal(
    settings: &Settings,
    request_kwargs: Option<&RequestKwargs>,
    resolved_messages: Option<&[ChatMessage]>,
) -> Result<Option<String>, ClassifierTextError> {
    if settings.plan_mode_min_tier.is_none() {
        return Ok(None);
    }
    let configured = settings.reminder_markers.as_deref();
    let markers: Option<Vec<MarkerPair<'_>>> = configured.map(|pairs| {
        pairs
            .iter()
            .map(|pair| MarkerPair {
                open: pair.open.as_str(),
                close: pair.close.as_str(),
            })
            .collect()
    });
    let active = reminder_markers_for_request(markers.as_deref(), codex_user_agent(request_kwargs));
    matched_plan_mode_sentinel(
        plan_mode_body(request_kwargs),
        resolved_messages,
        &settings.plan_mode_patterns,
        active,
    )
}

// _reminder_markers_for_request reads the client prefix from litellm_metadata before metadata
fn codex_user_agent(request_kwargs: Option<&RequestKwargs>) -> bool {
    let Some(kwargs) = request_kwargs else {
        return false;
    };
    for key in ["litellm_metadata", "metadata"] {
        let Some(Json::Object(metadata)) = kwargs.get(key) else {
            continue;
        };
        let Some(Json::Text(user_agent)) = metadata.get("user_agent") else {
            continue;
        };
        if is_codex_user_agent(user_agent) {
            return true;
        }
    }
    false
}

fn plan_mode_body(request_kwargs: Option<&RequestKwargs>) -> Option<&Json> {
    let Some(Json::Object(request)) = request_kwargs?.get("proxy_server_request") else {
        return None;
    };
    match request.get("body") {
        Some(body @ Json::Object(_)) => Some(body),
        _ => None,
    }
}
