use std::collections::BTreeMap;

use betterlite_error::proxy_error_types::ModelAccessObject;

use crate::auth_model_patterns::ALL_PROXY_MODELS;
use crate::auth_model_patterns::denial::{ModelAccessDecision, model_access_denied};
use crate::auth_model_patterns::grant::{
    ModelGrantPattern, model_in_team_aliases, team_alias_target,
};
use crate::auth_model_patterns::pattern::{ProviderInference, matches_wildcard_patterns};
use crate::auth_model_patterns::sentinel::resolve_team_model_sentinel;
use crate::auth_model_patterns::view::ModelAccessView;
use crate::auth_request_models::RequestedModels;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionContext<'request> {
    pub parent_model: &'request str,
    pub compactor: &'request str,
}

impl<'request> CompactionContext<'request> {
    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub fn parent_for(&self, model: &str) -> Option<&'request str> {
        (self.compactor == model).then_some(self.parent_model)
    }
}

pub struct ModelAccessContext<'context> {
    pub view: Option<&'context dyn ModelAccessView>,
    pub global_model_aliases: &'context BTreeMap<String, String>,
    pub compaction: Option<CompactionContext<'context>>,
    pub provider: &'context dyn ProviderInference,
    pub max_recurse_depth: i64,
}

#[must_use]
pub fn check_model_access(
    model: &str,
    grant: &ModelGrantPattern,
    context: &ModelAccessContext<'_>,
) -> bool {
    let team_id = grant.team_id.as_deref();
    let access_groups = context
        .view
        .map(|view| view.access_groups(model, team_id))
        .unwrap_or_default();
    let models = resolve_team_model_sentinel(&grant.models, context.view, team_id);
    // without a view there are no access groups, so a non-empty map already means the router answered
    if !access_groups.is_empty()
        && models
            .iter()
            .any(|allowed| access_groups.contains_key(allowed))
    {
        return true;
    }
    let filtered: Vec<String> = models
        .iter()
        .filter(|allowed| !access_groups.contains_key(*allowed))
        .cloned()
        .collect();
    if model_in_team_aliases(model, &grant.team_model_aliases) {
        return true;
    }
    if matches_wildcard_patterns(model, &filtered, context.provider) {
        return true;
    }
    // filtering only removes names the map holds, so an empty allowlist stays the only unrestricted case
    let all_model_access = models.is_empty()
        || filtered.iter().any(|allowed| allowed.as_str() == "*")
        || filtered
            .iter()
            .any(|allowed| allowed.as_str() == ALL_PROXY_MODELS);
    all_model_access || filtered.iter().any(|allowed| allowed.as_str() == model)
}

#[must_use]
pub fn can_object_call_model(
    model: &RequestedModels,
    grant: &ModelGrantPattern,
    object: ModelAccessObject,
    context: &ModelAccessContext<'_>,
    fallback_depth: i64,
) -> ModelAccessDecision {
    if fallback_depth >= context.max_recurse_depth {
        let received = match model {
            RequestedModels::None => String::new(),
            RequestedModels::One(name) => name.clone(),
            RequestedModels::Many(names) => names.join(","),
        };
        return ModelAccessDecision::RecurseDepthExceeded { model: received };
    }
    match model {
        RequestedModels::None => ModelAccessDecision::Allowed,
        RequestedModels::Many(names) => {
            for name in names {
                let decision = can_object_call_model(
                    &RequestedModels::One(name.clone()),
                    grant,
                    object,
                    context,
                    fallback_depth.saturating_add(1),
                );
                if !decision.is_allowed() {
                    return decision;
                }
            }
            ModelAccessDecision::Allowed
        }
        RequestedModels::One(name) => check_one(name, grant, object, context),
    }
}

fn check_one(
    model: &str,
    grant: &ModelGrantPattern,
    object: ModelAccessObject,
    context: &ModelAccessContext<'_>,
) -> ModelAccessDecision {
    if potential_models(model, grant, context)
        .iter()
        .any(|potential| check_model_access(potential, grant, context))
    {
        return ModelAccessDecision::Allowed;
    }
    ModelAccessDecision::Denied(Box::new(model_access_denied(object, &grant.models, model)))
}

// a request may be granted by its own name, a compaction parent or an alias target; once a key alias rewrites the name, only the dispatched target is checked
fn potential_models(
    model: &str,
    grant: &ModelGrantPattern,
    context: &ModelAccessContext<'_>,
) -> Vec<String> {
    let after_team = team_alias_target(model, &grant.team_model_aliases).to_owned();
    let after_key = aliased(&grant.key_model_aliases, &after_team);
    let after_global = aliased(context.global_model_aliases, &after_key);
    let dispatched = aliased(&grant.key_model_aliases, &after_global);
    if after_key != after_team || dispatched != after_global {
        return vec![dispatched];
    }
    let mut potential = vec![model.to_owned()];
    if let Some(parent) = context
        .compaction
        .and_then(|compaction| compaction.parent_for(model))
    {
        potential.push(parent.to_owned());
    }
    if let Some(target) = global_alias_target(model, context) {
        potential.push(target);
    }
    potential
}

// the global map decides by presence: an empty target there is no alias, and the router is only consulted without an entry
fn global_alias_target(model: &str, context: &ModelAccessContext<'_>) -> Option<String> {
    if let Some(target) = context.global_model_aliases.get(model) {
        return Some(target.clone()).filter(|target_name| !target_name.is_empty());
    }
    let view = context.view?;
    view.model_group_alias_target(model)
}

// _resolve_key_models_for_auth_check reads a missing alias as the name itself
fn aliased(aliases: &BTreeMap<String, String>, model: &str) -> String {
    aliases
        .get(model)
        .cloned()
        .unwrap_or_else(|| model.to_owned())
}
