use std::collections::{BTreeMap, BTreeSet};

use self::fields::{args_field, models_field, priorities_field, text_field};
use crate::router_predicates::JsonMap;
use crate::routing_groups::error::RoutingGroupError;
use crate::strategy_dispatch::groups::GroupStrategy;
use crate::strategy_dispatch::strategy::RoutingStrategy;

mod fields;

// RoutingGroup.model_priorities' bound, pydantic's le=2**53-1.
pub const MAX_MODEL_PRIORITY: u64 = 0x001F_FFFF_FFFF_FFFF;
pub const PRIORITY_STRATEGY: &str = "priority";
const RESERVED_GROUP_NAME: &str = "default";

// One validated entry of router_settings.routing_groups.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoutingGroup {
    pub group_name: String,
    pub models: Vec<String>,
    pub routing_strategy: GroupStrategy,
    pub routing_strategy_args: Option<JsonMap>,
    pub model_priorities: Option<BTreeMap<String, u64>>,
}

// The groups parse_routing_groups returns plus the unknown-member warnings it logs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParseOutcome {
    pub groups: Vec<RoutingGroup>,
    pub warnings: Vec<String>,
}

// A group before its strategy name is typed, so field and schema errors precede every name check.
struct RawGroup {
    group_name: String,
    models: Vec<String>,
    routing_strategy: String,
    routing_strategy_args: Option<JsonMap>,
    model_priorities: Option<BTreeMap<String, u64>>,
}

impl RoutingGroup {
    pub fn new(
        group_name: String,
        models: Vec<String>,
        routing_strategy: &str,
        routing_strategy_args: Option<JsonMap>,
        model_priorities: Option<BTreeMap<String, u64>>,
    ) -> Result<Self, RoutingGroupError> {
        RawGroup {
            group_name,
            models,
            routing_strategy: routing_strategy.to_owned(),
            routing_strategy_args,
            model_priorities,
        }
        .schema_checked()?
        .resolve_strategy()
    }
}

impl RawGroup {
    // RoutingGroup(**raw): pydantic's field types, then its _validate_model_priorities.
    fn from_raw(raw: &JsonMap) -> Result<Self, RoutingGroupError> {
        Ok(Self {
            group_name: text_field(raw, "group_name")?,
            models: models_field(raw)?,
            routing_strategy: text_field(raw, "routing_strategy")?,
            routing_strategy_args: args_field(raw)?,
            model_priorities: priorities_field(raw)?,
        })
    }

    fn schema_checked(self) -> Result<Self, RoutingGroupError> {
        if self.routing_strategy != PRIORITY_STRATEGY {
            if self
                .model_priorities
                .as_ref()
                .is_some_and(|priorities| !priorities.is_empty())
            {
                return Err(RoutingGroupError::PrioritiesRequirePriority);
            }
            return Ok(self);
        }
        let distinct: BTreeSet<&str> = self.models.iter().map(String::as_str).collect();
        if self.models.is_empty() || distinct.len() != self.models.len() {
            return Err(RoutingGroupError::PriorityModels);
        }
        let Some(priorities) = &self.model_priorities else {
            return Err(RoutingGroupError::PriorityMembership);
        };
        if priorities
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            != distinct
        {
            return Err(RoutingGroupError::PriorityMembership);
        }
        if self
            .routing_strategy_args
            .as_ref()
            .is_some_and(|args| !args.is_empty())
        {
            return Err(RoutingGroupError::PriorityArgs);
        }
        Ok(self)
    }

    // validate_routing_strategy for the groups that route, then the typed strategy
    fn resolve_strategy(self) -> Result<RoutingGroup, RoutingGroupError> {
        let routing_strategy = if self.routing_strategy == PRIORITY_STRATEGY {
            GroupStrategy::Priority
        } else {
            GroupStrategy::Routing(RoutingStrategy::parse(&self.routing_strategy)?)
        };
        Ok(RoutingGroup {
            group_name: self.group_name,
            models: self.models,
            routing_strategy,
            routing_strategy_args: self.routing_strategy_args,
            model_priorities: self.model_priorities,
        })
    }
}

// validate_routing_strategy: None passes, and only the strategy names LiteLLM lists are accepted.
pub fn validate_routing_strategy(value: Option<&str>) -> Result<(), RoutingGroupError> {
    match value {
        None => Ok(()),
        Some(name) => RoutingStrategy::parse(name)
            .map(|_| ())
            .map_err(RoutingGroupError::from),
    }
}

// parse_routing_groups: pydantic's constructor on every entry, then the pinned name, strategy, ownership and unknown-member checks in order.
pub fn parse_routing_groups(
    groups_input: Option<&[JsonMap]>,
    known_model_names: &BTreeSet<String>,
) -> Result<ParseOutcome, RoutingGroupError> {
    let raw = groups_input.unwrap_or_default();
    let mut checked = Vec::with_capacity(raw.len());
    for group in raw {
        checked.push(RawGroup::from_raw(group)?.schema_checked()?);
    }
    if checked.is_empty() {
        return Ok(ParseOutcome::default());
    }
    check_names(&checked)?;
    for group in &checked {
        if group.routing_strategy != PRIORITY_STRATEGY {
            validate_routing_strategy(Some(&group.routing_strategy))?;
        }
    }
    let mut groups = Vec::with_capacity(checked.len());
    for group in checked {
        groups.push(group.resolve_strategy()?);
    }
    check_ownership(&groups)?;
    Ok(ParseOutcome {
        warnings: unknown_member_warnings(&groups, known_model_names),
        groups,
    })
}

fn check_names(groups: &[RawGroup]) -> Result<(), RoutingGroupError> {
    if groups.iter().any(|group| group.group_name.is_empty()) {
        return Err(RoutingGroupError::EmptyName);
    }
    if groups
        .iter()
        .any(|group| group.group_name == RESERVED_GROUP_NAME)
    {
        return Err(RoutingGroupError::ReservedDefault);
    }
    let mut seen = BTreeSet::new();
    let mut duplicates = BTreeSet::new();
    for group in groups {
        if !seen.insert(group.group_name.as_str()) {
            duplicates.insert(group.group_name.as_str());
        }
    }
    match duplicates.iter().next() {
        Some(duplicate) => Err(RoutingGroupError::DuplicateName {
            group: (*duplicate).to_owned(),
        }),
        None => Ok(()),
    }
}

fn check_ownership(groups: &[RoutingGroup]) -> Result<(), RoutingGroupError> {
    let mut members: Vec<&str> = Vec::new();
    for group in groups {
        for model in &group.models {
            if !members.contains(&model.as_str()) {
                members.push(model.as_str());
            }
        }
    }
    let mut conflicts = Vec::new();
    for model in members {
        let owners: Vec<String> = groups
            .iter()
            .filter(|group| {
                !matches!(group.routing_strategy, GroupStrategy::Priority)
                    && group.models.iter().any(|member| member == model)
            })
            .map(|group| format!("'{}'", group.group_name))
            .collect();
        if owners.len() > 1 {
            conflicts.push(format!(
                "model_name '{model}' appears in {}",
                owners.join(" and ")
            ));
        }
    }
    if conflicts.is_empty() {
        return Ok(());
    }
    Err(RoutingGroupError::ModelOwnerConflict {
        conflicts: conflicts.join("; "),
    })
}

fn unknown_member_warnings(
    groups: &[RoutingGroup],
    known_model_names: &BTreeSet<String>,
) -> Vec<String> {
    if known_model_names.is_empty() {
        return Vec::new();
    }
    let mut warnings = Vec::new();
    for group in groups {
        for model in &group.models {
            if !known_model_names.contains(model) {
                warnings.push(format!(
                    "routing_groups: model_name '{model}' (group '{}') is not in model_list; the group entry will only take effect once a deployment with that model_name is added.",
                    group.group_name
                ));
            }
        }
    }
    warnings
}
