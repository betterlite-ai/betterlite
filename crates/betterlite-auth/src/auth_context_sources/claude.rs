use std::collections::BTreeMap;

use betterlite_db::identity_rows::Payload;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::auth_request_models::RequestedModels;
use crate::auth_request_models::catalog::{CatalogError, Collaborators};

use super::context::{ModelContext, get_model_from_request_context};
use super::router_settings::{TeamRouterSettings, hierarchical_router_settings};

pub const CLAUDE_MODEL_NORMALIZED_MARKER: &str = "litellm.claude_model_normalized";
pub const CLAUDE_ROUTER_PREFIX: &str = "claude-router-";
pub const ONE_MILLION_SUFFIX: &str = "[1m]";

// _CLAUDE_MODEL_ROUTES: the Anthropic-shaped endpoints Claude Code sends its chosen model to.
pub const CLAUDE_MODEL_ROUTES: [&str; 6] = [
    "/messages",
    "/v1/messages",
    "/chat/completions",
    "/v1/chat/completions",
    "/responses",
    "/v1/responses",
];

// The alias maps a request carries: a key's aliases, its team's aliases and the hierarchical settings.
#[derive(Clone, Copy, Debug)]
pub enum AliasMap<'map> {
    Payload(&'map Payload),
    Json(&'map Json),
}

impl AliasMap<'_> {
    fn contains_key(&self, name: &str) -> bool {
        match self {
            Self::Payload(aliases) => aliases.0.contains_key(name),
            Self::Json(Json::Object(entries)) => entries.contains_key(name),
            Self::Json(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_),
            ) => false,
        }
    }
}

// The router questions ClaudeCodeRoutingNames asks; B302's router answers them with its live deployments.
pub trait RouterNames {
    fn model_group_alias_contains(&self, name: &str) -> bool;
    fn has_model_id(&self, name: &str) -> bool;
    fn candidate_model_ids_for_route(&self, name: &str, team_id: Option<&str>) -> Vec<String>;
}

pub trait RoutingNames {
    fn contains(&self, name: &str) -> bool;
}

// ClaudeCodeRoutingNames: configured routes always own their names, including aliases and wildcard routes.
pub struct ClaudeNameSources<'names> {
    pub global_aliases: &'names BTreeMap<String, String>,
    pub key_aliases: Option<AliasMap<'names>>,
    pub team_model_aliases: Option<AliasMap<'names>>,
    pub hierarchical_model_group_alias: Option<AliasMap<'names>>,
    pub router: Option<&'names dyn RouterNames>,
    pub team_id: Option<&'names str>,
}

impl RoutingNames for ClaudeNameSources<'_> {
    fn contains(&self, name: &str) -> bool {
        if self.global_aliases.contains_key(name) {
            return true;
        }
        let alias_maps = [
            self.key_aliases,
            self.team_model_aliases,
            self.hierarchical_model_group_alias,
        ];
        if alias_maps
            .into_iter()
            .flatten()
            .any(|aliases| aliases.contains_key(name))
        {
            return true;
        }
        self.router.is_some_and(|router| {
            router.model_group_alias_contains(name)
                || router.has_model_id(name)
                || !router
                    .candidate_model_ids_for_route(name, self.team_id)
                    .is_empty()
        })
    }
}

// claude_code_requested_group: the group a Claude Code compatibility id names, or None when no route owns it.
#[must_use]
pub fn claude_code_requested_group(requested: &str, names: &dyn RoutingNames) -> Option<String> {
    if names.contains(requested) {
        return None;
    }
    let unmarked = unmark_one_million(requested);
    if unmarked != requested && names.contains(unmarked) {
        return Some(unmarked.to_owned());
    }
    let model_id = decoded_compatibility_id(requested)?;
    if model_id.is_empty() || !names.contains(&model_id) {
        return None;
    }
    Some(model_id)
}

// _unmarked: the [1m] marker is case-insensitive and only ever removed from the end.
fn unmark_one_million(name: &str) -> &str {
    let Some(start) = name.len().checked_sub(ONE_MILLION_SUFFIX.len()) else {
        return name;
    };
    let Some(tail) = name.get(start..) else {
        return name;
    };
    if !tail.eq_ignore_ascii_case(ONE_MILLION_SUFFIX) {
        return name;
    }
    name.get(..start).unwrap_or_default()
}

// _decoded_compatibility_id: only a canonical claude-router-<hex> id decodes, so a lookalike is left alone.
fn decoded_compatibility_id(view_id: &str) -> Option<String> {
    let unmarked = unmark_one_million(view_id);
    let encoded = unmarked.strip_prefix(CLAUDE_ROUTER_PREFIX)?;
    let model_id = String::from_utf8(hex::decode(encoded).ok()?).ok()?;
    let canonical = format!("{CLAUDE_ROUTER_PREFIX}{}", hex::encode(model_id.as_bytes()));
    if canonical != unmarked {
        return None;
    }
    Some(model_id)
}

pub struct ClaudeNormalizationInput<'request> {
    pub route: &'request str,
    pub request_data: &'request JsonObject,
    pub model_context: ModelContext<'request>,
    pub collaborators: &'request Collaborators<'request>,
    pub router: Option<&'request dyn RouterNames>,
    pub team_id: Option<&'request str>,
    pub global_aliases: &'request BTreeMap<String, String>,
    pub key_aliases: Option<AliasMap<'request>>,
    pub team_model_aliases: Option<AliasMap<'request>>,
    pub key_router_settings: Option<&'request Json>,
    pub normalized: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeNormalization {
    pub normalized: bool,
    pub source: Option<String>,
}

// _normalize_claude_model up to the request mutation: the caller stores `normalized` and applies `source`.
pub async fn normalize_claude_model(
    input: &ClaudeNormalizationInput<'_>,
    teams: &dyn TeamRouterSettings,
) -> Result<ClaudeNormalization, CatalogError> {
    if !CLAUDE_MODEL_ROUTES.contains(&input.route) || input.router.is_none() {
        return Ok(ClaudeNormalization {
            normalized: input.normalized,
            source: None,
        });
    }
    if input.normalized {
        return Ok(ClaudeNormalization {
            normalized: true,
            source: None,
        });
    }
    let Some(requested) = eligible_requested_model(input)? else {
        return Ok(ClaudeNormalization {
            normalized: false,
            source: None,
        });
    };
    let source = requested_group(input, &requested, teams).await;
    Ok(ClaudeNormalization {
        normalized: true,
        source,
    })
}

// The resolved model must be one string identical to body.model and shaped like a compatibility id.
fn eligible_requested_model(
    input: &ClaudeNormalizationInput<'_>,
) -> Result<Option<String>, CatalogError> {
    let requested = get_model_from_request_context(&input.model_context, input.collaborators)?;
    let RequestedModels::One(requested) = requested else {
        return Ok(None);
    };
    if input.request_data.get("model") != Some(&Json::Text(requested.clone())) {
        return Ok(None);
    }
    if !requested.starts_with(CLAUDE_ROUTER_PREFIX)
        && !requested.to_lowercase().ends_with(ONE_MILLION_SUFFIX)
    {
        return Ok(None);
    }
    Ok(Some(requested))
}

// claude_code_requested_group with the router, its team and the three alias maps, in that order.
async fn requested_group(
    input: &ClaudeNormalizationInput<'_>,
    requested: &str,
    teams: &dyn TeamRouterSettings,
) -> Option<String> {
    let settings =
        hierarchical_router_settings(input.key_router_settings, input.team_id, teams).await;
    let hierarchical = settings
        .as_ref()
        .and_then(|resolved| resolved.get("model_group_alias"));
    let names = ClaudeNameSources {
        global_aliases: input.global_aliases,
        key_aliases: input.key_aliases,
        team_model_aliases: input.team_model_aliases,
        hierarchical_model_group_alias: hierarchical.map(AliasMap::Json),
        router: input.router,
        team_id: input.team_id,
    };
    claude_code_requested_group(requested, &names)
}
