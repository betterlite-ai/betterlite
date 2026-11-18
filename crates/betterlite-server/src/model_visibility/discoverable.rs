use std::collections::BTreeSet;

use betterlite_auth::resolved_key::auth::{UserAPIKeyAuth, user_api_key_has_admin_view};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::alias::{AliasMap, AliasTarget};
use betterlite_router::model_catalog_lookup::error::LookupError;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::wildcard_catalog::PatternCatalog;

// The wildcard names a hidden row registers, split the way LiteLLM splits its two pattern routers.
struct HiddenPatterns {
    global: Vec<String>,
    team: Vec<String>,
}

// A model_info text field: an absent, null or non-text value reads as none.
fn model_info_text<'info>(model_info: &'info JsonObject, field: &str) -> Option<&'info str> {
    match model_info.get(field) {
        Some(Json::Text(value)) => Some(value.as_str()),
        _ => None,
    }
}

// _alias_target: the group a string alias or an item's model names, ignoring the item's hidden flag.
const fn alias_target_name(target: &AliasTarget) -> Option<&str> {
    match target {
        AliasTarget::Name(name) => Some(name.as_str()),
        AliasTarget::Item {
            model: Some(Json::Text(model)),
            ..
        } => Some(model.as_str()),
        AliasTarget::Item { .. } | AliasTarget::Unsupported(_) => None,
    }
}

// _undiscoverable_served_names: the names and team public names hidden rows serve, plus the aliases whose direct target is one of them.
fn served_names(rows: &[&DeploymentSpec], aliases: &AliasMap) -> BTreeSet<String> {
    let mut served: BTreeSet<String> = BTreeSet::new();
    for row in rows {
        served.insert(row.model_name.clone());
        if let Some(public_name) = model_info_text(&row.model_info, "team_public_model_name") {
            served.insert(public_name.to_owned());
        }
    }
    let hidden_aliases: Vec<String> = aliases
        .iter()
        .filter(|(_alias, target)| {
            alias_target_name(target).is_some_and(|name| served.contains(name))
        })
        .map(|(alias, _target)| alias.clone())
        .collect();
    served.extend(hidden_aliases);
    served
}

// _undiscoverable_patterns: the pattern names whose registry holds a hidden deployment, as the key each wildcard router compiled.
fn hidden_pattern_names(catalog: &DeploymentCatalog, team_id: Option<&str>) -> HiddenPatterns {
    let mut hidden = HiddenPatterns {
        global: Vec::new(),
        team: Vec::new(),
    };
    for row in &catalog.models {
        if !matches!(row.model_info.get("discoverable"), Some(Json::Bool(false))) {
            continue;
        }
        hidden.global.push(row.model_name.clone());
        if let Some(public_name) = model_info_text(&row.model_info, "team_public_model_name")
            && model_info_text(&row.model_info, "team_id") == team_id
        {
            hidden.team.push(public_name.to_owned());
        }
    }
    hidden
}

// _matches_undiscoverable_pattern: the name, then the declared or inferred provider and a slash before it, against both hidden pattern sets.
fn matches_hidden_pattern(
    name: &str,
    lookup: &CatalogLookup<'_>,
    team_id: Option<&str>,
    hidden: &HiddenPatterns,
) -> bool {
    let team: Option<&PatternCatalog> = team_id
        .and_then(|team| lookup.catalog.team_pattern_routers.get(team))
        .map(|patterns| &patterns.router);
    let matched = |request: &str| {
        lookup
            .catalog
            .pattern_router
            .route(request, Some(hidden.global.as_slice()))
            .is_some()
            || team.is_some_and(|router| {
                router
                    .route(request, Some(hidden.team.as_slice()))
                    .is_some()
            })
    };
    if matched(name) {
        return true;
    }
    let provider = (lookup.inferred_provider)(name);
    let qualified = format!("{}/{name}", provider.as_deref().unwrap_or("None"));
    matched(&qualified)
}

// is_undiscoverable_model_name: a name with at least one deployment, every one of them hidden.
fn is_undiscoverable_model_name(
    model_name: &str,
    lookup: &CatalogLookup<'_>,
    team_id: Option<&str>,
) -> Result<bool, LookupError> {
    let rows = lookup.get_model_list(Some(model_name), team_id)?;
    Ok(!rows.is_empty()
        && rows
            .iter()
            .all(|row| matches!(row.model_info.get("discoverable"), Some(Json::Bool(false)))))
}

// undiscoverable_model_names: the listed names a caller without the admin view may not see; a miss, a visible sibling or no hidden row keeps a name listed.
pub fn undiscoverable_model_names(
    model_names: &[String],
    router: Option<&CatalogLookup<'_>>,
    user_api_key_dict: &UserAPIKeyAuth,
    team_id: Option<&str>,
) -> Result<BTreeSet<String>, LookupError> {
    if user_api_key_has_admin_view(user_api_key_dict) {
        return Ok(BTreeSet::new());
    }
    let Some(lookup) = router else {
        return Ok(BTreeSet::new());
    };
    let listed = lookup.get_model_list(None, None)?;
    let hidden: Vec<&DeploymentSpec> = listed
        .iter()
        .filter(|row| matches!(row.model_info.get("discoverable"), Some(Json::Bool(false))))
        .collect();
    if hidden.is_empty() {
        return Ok(BTreeSet::new());
    }
    let served = served_names(&hidden, lookup.model_group_alias);
    let patterns = hidden_pattern_names(lookup.catalog, team_id);
    let mut names = BTreeSet::new();
    for name in model_names {
        let candidate =
            served.contains(name) || matches_hidden_pattern(name, lookup, team_id, &patterns);
        if candidate && is_undiscoverable_model_name(name, lookup, team_id)? {
            names.insert(name.clone());
        }
    }
    Ok(names)
}

// discoverable_rows: every row for an admin view, and the unmarked rows in order, duplicates kept, for everyone else.
#[must_use]
pub fn discoverable_rows<'row>(
    rows: &'row [DeploymentSpec],
    user_api_key_dict: &UserAPIKeyAuth,
) -> Vec<&'row DeploymentSpec> {
    let admin = user_api_key_has_admin_view(user_api_key_dict);
    rows.iter()
        .filter(|row| {
            admin || !matches!(row.model_info.get("discoverable"), Some(Json::Bool(false)))
        })
        .collect()
}
