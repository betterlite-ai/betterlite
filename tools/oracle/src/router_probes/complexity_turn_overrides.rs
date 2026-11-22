use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatMessage;
use betterlite_router::complexity_scoring::tier::Tier;
use betterlite_router::complexity_turn_overrides::keywords::LexicalRules;
use betterlite_router::complexity_turn_overrides::plan::{
    apply_plan_mode_floor, plan_mode_floor, plan_mode_floor_is_top_tier,
};
use betterlite_router::complexity_turn_overrides::semantic::SemanticKeywordIndex;
use betterlite_router::complexity_turn_overrides::settings::{Settings, active_tier};
use betterlite_router::complexity_turn_overrides::turn::{
    TurnOverrides, TurnRequest, resolve_turn,
};
use betterlite_router::semantic_router::service::CallerContext;
use betterlite_router::tier_definitions::model::TierSet;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;

use self::setup::{build_catalog, build_settings, semantic_index};
use self::table::TableEmbedder;
use self::wire::{Fields, MarkerInput, OverrideOut, RuleInput, count, invalid};
use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

mod setup;
mod table;
mod wire;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    #[serde(default)]
    tiers: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tier_definitions: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    non_reasoning: Option<bool>,
    #[serde(default)]
    keyword_tier_rules: Vec<RuleInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    semantic_keyword_matching: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    embedding_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    match_threshold: Option<Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stall_escalation_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stall_escalation_window: Option<Json>,
    #[serde(
        default,
        rename = "stall_escalation_repeat_threshold",
        skip_serializing_if = "Option::is_none"
    )]
    stall_repeat_threshold: Option<Json>,
    #[serde(
        default,
        rename = "route_housekeeping_to_cheapest_tier",
        skip_serializing_if = "Option::is_none"
    )]
    housekeeping_cheapest: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    escalation_keywords: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plan_mode_min_tier: Option<String>,
    #[serde(default)]
    plan_mode_patterns: Vec<String>,
    #[serde(default)]
    housekeeping_patterns: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reminder_markers: Option<Vec<MarkerInput>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    classifier_type: Option<String>,
    #[serde(default)]
    messages: Vec<Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request_kwargs: Option<Json>,
    #[serde(default)]
    user_message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    newest_ask: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    decided_tier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    turns: Option<Json>,
    #[serde(default)]
    embeddings: BTreeMap<String, Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    document_failures: Option<Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    query_failures: Option<Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Output(Box<Fields>);

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let prepared = prepare(input)?;
    let found = run_turns(input, &prepared)?;
    let observed = fields(&prepared, &found)?;
    Ok(ProbeOutput::ComplexityTurnOverrides(Output(Box::new(
        observed,
    ))))
}

struct Prepared {
    catalog: TierSet,
    settings: Settings,
    lexical: LexicalRules,
    semantic: Option<SemanticKeywordIndex<TableEmbedder>>,
    embedder: Arc<TableEmbedder>,
    parsed_messages: Option<Vec<ChatMessage>>,
    request_kwargs: Option<IndexMap<String, Json>>,
    decided: Option<Tier>,
}

fn prepare(input: &Input) -> Result<Prepared, OperationError> {
    let catalog = build_catalog(input);
    let settings = build_settings(input)?;
    let lexical = LexicalRules::compile(&catalog, &settings.keyword_tier_rules)
        .map_err(|error| invalid(&error.to_string()))?;
    let decided = input
        .decided_tier
        .as_deref()
        .map(|name| active_tier(&catalog, name))
        .transpose()
        .map_err(|error| invalid(&error.to_string()))?;
    let embedder = Arc::new(TableEmbedder::new(input)?);
    let semantic = semantic_index(input, &settings, &catalog, &embedder);
    Ok(Prepared {
        catalog,
        settings,
        lexical,
        semantic,
        embedder,
        parsed_messages: parsed_messages(input)?,
        request_kwargs: request_kwargs(input)?,
        decided,
    })
}

fn run_turns(input: &Input, prepared: &Prepared) -> Result<TurnOverrides, OperationError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| invalid(&source.to_string()))?;
    let raw_messages = if input.messages.is_empty() {
        None
    } else {
        Some(input.messages.as_slice())
    };
    let turns = count(input.turns.as_ref(), 1)?;
    let mut last = None;
    for _turn in 0..turns {
        let request = TurnRequest {
            request_kwargs: prepared.request_kwargs.as_ref(),
            resolved_messages: prepared.parsed_messages.as_deref(),
            messages: raw_messages,
            user_message: &input.user_message,
            newest_ask: input.newest_ask.as_deref(),
            caller: &CallerContext::default(),
        };
        let found = runtime
            .block_on(resolve_turn(
                &prepared.settings,
                &prepared.catalog,
                &prepared.lexical,
                prepared.semantic.as_ref(),
                &request,
            ))
            .map_err(|error| invalid(&error.to_string()))?;
        last = Some(found);
    }
    last.ok_or_else(|| invalid("the probe needs at least one turn"))
}

fn fields(prepared: &Prepared, found: &TurnOverrides) -> Result<Fields, OperationError> {
    let catalog = &prepared.catalog;
    let settings = &prepared.settings;
    let floor = plan_mode_floor(settings, catalog)
        .map_err(|error| invalid(&error.to_string()))?
        .map(|tier| tier.name().to_owned());
    let top = plan_mode_floor_is_top_tier(settings, catalog)
        .map_err(|error| invalid(&error.to_string()))?;
    let applied = prepared
        .decided
        .as_ref()
        .map(|tier| apply_plan_mode_floor(settings, catalog, tier.clone()))
        .transpose()
        .map_err(|error| invalid(&error.to_string()))?
        .map(|tier| tier.name().to_owned());
    Ok(Fields {
        plan_mode_sentinel: found.plan_mode_sentinel.clone(),
        plan_mode_floor: floor,
        plan_mode_floor_is_top_tier: top,
        applied_plan_mode_floor: applied,
        escalation_keyword: found.escalation_keyword.clone(),
        stalled: found.stalled,
        housekeeping_sentinel: found.housekeeping_sentinel.clone(),
        housekeeping_tier: found
            .housekeeping_tier
            .as_ref()
            .map(|tier| tier.name().to_owned()),
        keyword_override: found.keyword_override.as_ref().map(|matched| OverrideOut {
            tier: matched.tier.name().to_owned(),
            matched_keyword: matched.matched_keyword.clone(),
        }),
        embedding_calls: prepared.embedder.calls(),
    })
}

fn parsed_messages(input: &Input) -> Result<Option<Vec<ChatMessage>>, OperationError> {
    if input.messages.is_empty() {
        return Ok(None);
    }
    let mut messages = Vec::with_capacity(input.messages.len());
    for message in &input.messages {
        messages.push(
            serde_json::to_value(message)
                .and_then(serde_json::from_value::<ChatMessage>)
                .map_err(|source| invalid(&source.to_string()))?,
        );
    }
    Ok(Some(messages))
}

fn request_kwargs(input: &Input) -> Result<Option<IndexMap<String, Json>>, OperationError> {
    let Some(kwargs) = input.request_kwargs.as_ref() else {
        return Ok(None);
    };
    let Json::Object(fields) = kwargs else {
        return Err(invalid("request_kwargs is not an object"));
    };
    Ok(Some(fields.clone()))
}
