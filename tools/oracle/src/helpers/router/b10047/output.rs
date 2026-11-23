use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::model_group_lookups::access_groups::AccessGroups;
use betterlite_router::routing_groups::index::RoutingGroupIndex;
use serde_json::{Value, json};

use super::error::Failure;
use super::input::Input;
use super::snapshot;

// The router infers no provider for a case, so a wildcard lookup tries the bare request name only.
const fn no_inferred(_model: &str) -> Option<String> {
    None
}

// get_model_access_groups over a fresh cache, as the case's arguments ask; the map itself is the answer.
pub(crate) fn answer(input: &Input) -> Result<Value, Failure> {
    let catalog = snapshot::catalog(&input.model_list)?;
    let aliases = snapshot::alias_map(&input.model_group_alias);
    let groups = RoutingGroupIndex::default();
    let lookup = CatalogLookup {
        catalog: &catalog,
        model_group_alias: &aliases,
        routing_groups: &groups,
        inferred_provider: &no_inferred,
    };
    let arguments = &input.arguments;
    let map = AccessGroups::default().get_model_access_groups(
        &lookup,
        arguments.model_name.as_deref(),
        arguments.model_access_group.as_deref(),
        arguments.team_id.as_deref(),
    )?;
    Ok(json!(map))
}
