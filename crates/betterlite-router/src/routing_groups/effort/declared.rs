use betterlite_models::decisions::Json;
use betterlite_models::metadata::levels::ReasoningEffort;
use betterlite_models::model_metadata_resolution::names::ModelCostMap;

use crate::router_predicates::JsonMap;

// REASONING_EFFORT_ADVERTISEMENT_ORDER: presentation order, not a strength scale.
pub const ADVERTISEMENT_ORDER: [ReasoningEffort; 7] = [
    ReasoningEffort::None,
    ReasoningEffort::Minimal,
    ReasoningEffort::Low,
    ReasoningEffort::Medium,
    ReasoningEffort::High,
    ReasoningEffort::Xhigh,
    ReasoningEffort::Max,
];

const DECLARED_EFFORTS_KEY: &str = "reasoning_effort_levels";

// One per-level flag, read with Python's identity checks on the raw value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EffortFlag {
    #[default]
    Absent,
    True,
    False,
    Other,
}

impl EffortFlag {
    pub(super) fn read(value: Option<&Json>) -> Self {
        match value {
            None | Some(Json::Null) => Self::Absent,
            Some(Json::Bool(true)) => Self::True,
            Some(Json::Bool(false)) => Self::False,
            Some(
                Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_),
            ) => Self::Other,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EffortFlags {
    pub none: EffortFlag,
    pub minimal: EffortFlag,
    pub low: EffortFlag,
    pub xhigh: EffortFlag,
    pub max: EffortFlag,
}

impl EffortFlags {
    pub(super) fn any_present(&self) -> bool {
        [self.none, self.minimal, self.low, self.xhigh, self.max]
            .into_iter()
            .any(|flag| !matches!(flag, EffortFlag::Absent))
    }

    pub(super) fn all_absent(&self) -> bool {
        [self.none, self.minimal, self.low, self.xhigh, self.max]
            .into_iter()
            .all(|flag| matches!(flag, EffortFlag::Absent))
    }

    pub(super) fn get(&self, effort: ReasoningEffort) -> EffortFlag {
        match effort {
            ReasoningEffort::None => self.none,
            ReasoningEffort::Minimal => self.minimal,
            ReasoningEffort::Low => self.low,
            ReasoningEffort::Xhigh => self.xhigh,
            ReasoningEffort::Max => self.max,
            ReasoningEffort::Medium | ReasoningEffort::High => EffortFlag::Absent,
        }
    }
}

// _bare_model_entry: the unprefixed twin of a provider-prefixed cost-map key.
#[must_use]
pub fn bare_model_entry(model_info: &JsonMap, model_cost: &ModelCostMap) -> JsonMap {
    let Some(key) = text(model_info, "key") else {
        return JsonMap::new();
    };
    let Some(provider) = text(model_info, "litellm_provider") else {
        return JsonMap::new();
    };
    let Some(stripped) = key.strip_prefix(&format!("{provider}/")) else {
        return JsonMap::new();
    };
    model_cost_entry(model_cost, stripped).unwrap_or_default()
}

// _declared_effort_flags: the entry's own flag, then the bare twin's, then nothing.
#[must_use]
pub fn declared_effort_flags(model_info: &JsonMap, model_cost: &ModelCostMap) -> EffortFlags {
    let bare = bare_model_entry(model_info, model_cost);
    let read = |flag: &str| {
        let own = EffortFlag::read(
            model_info
                .get(flag)
                .filter(|value| !matches!(value, Json::Null)),
        );
        if !matches!(own, EffortFlag::Absent) {
            own
        } else {
            EffortFlag::read(bare.get(flag).filter(|value| !matches!(value, Json::Null)))
        }
    };
    EffortFlags {
        none: read("supports_none_reasoning_effort"),
        minimal: read("supports_minimal_reasoning_effort"),
        low: read("supports_low_reasoning_effort"),
        xhigh: read("supports_xhigh_reasoning_effort"),
        max: read("supports_max_reasoning_effort"),
    }
}

// declared_reasoning_efforts: a present list is the whole answer, ordered for advertisement.
#[must_use]
pub fn declared_reasoning_efforts(
    model_info: &JsonMap,
    model_cost: &ModelCostMap,
) -> Option<Vec<ReasoningEffort>> {
    let bare = bare_model_entry(model_info, model_cost);
    let raw = match model_info
        .get(DECLARED_EFFORTS_KEY)
        .filter(|value| !matches!(value, Json::Null))
    {
        Some(value) => Some(value),
        None => bare
            .get(DECLARED_EFFORTS_KEY)
            .filter(|value| !matches!(value, Json::Null)),
    };
    let Some(Json::List(entries)) = raw else {
        return None;
    };
    let declared: Vec<ReasoningEffort> = entries
        .iter()
        .filter_map(|entry| match entry {
            Json::Text(name) => effort_from_wire(name),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        })
        .collect();
    Some(
        ADVERTISEMENT_ORDER
            .into_iter()
            .filter(|effort| declared.contains(effort))
            .collect(),
    )
}

// declared_efforts_for_model: the model string a provider config holds, not a deployment's id.
#[must_use]
pub fn declared_efforts_for_model(
    model: &str,
    custom_llm_provider: &str,
    model_cost: &ModelCostMap,
) -> Option<Vec<ReasoningEffort>> {
    let prefixed_key = format!("{custom_llm_provider}/{model}");
    if let Some(raw) = model_cost.entries.get(&prefixed_key) {
        let Ok(value) = serde_json::from_str::<Json>(raw.get()) else {
            return None;
        };
        let prefixed = match value {
            Json::Object(prefixed) => prefixed,
            Json::Null => JsonMap::new(),
            Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => return None,
        };
        if !prefixed.is_empty() {
            return declared_reasoning_efforts(&prefixed, model_cost);
        }
    }
    let entry = model_cost_entry(model_cost, model)?;
    declared_reasoning_efforts(&entry, model_cost)
}

pub(super) fn text<'map>(map: &'map JsonMap, key: &str) -> Option<&'map str> {
    match map.get(key).filter(|value| !matches!(value, Json::Null)) {
        Some(Json::Text(value)) => Some(value),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}

fn model_cost_entry(model_cost: &ModelCostMap, key: &str) -> Option<JsonMap> {
    let raw = model_cost.entries.get(key)?;
    match serde_json::from_str::<Json>(raw.get()).ok()? {
        Json::Object(entry) => Some(entry),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

pub fn effort_from_wire(name: &str) -> Option<ReasoningEffort> {
    ADVERTISEMENT_ORDER
        .into_iter()
        .find(|effort| effort_wire_name(*effort) == name)
}

pub const fn effort_wire_name(effort: ReasoningEffort) -> &'static str {
    match effort {
        ReasoningEffort::None => "none",
        ReasoningEffort::Minimal => "minimal",
        ReasoningEffort::Low => "low",
        ReasoningEffort::Medium => "medium",
        ReasoningEffort::High => "high",
        ReasoningEffort::Xhigh => "xhigh",
        ReasoningEffort::Max => "max",
    }
}
