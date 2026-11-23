use std::sync::PoisonError;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::cache::{CacheClear, CacheSlot};
use crate::model_catalog_lookup::lookup::CatalogLookup;

use super::error::AccessGroupError;

// Router.get_model_access_groups' answer: group name to the deployments that list it, in configuration order.
pub type AccessGroupMap = IndexMap<String, Vec<String>>;

// Router.get_model_access_groups and the no-args map it keeps for the hot path.
#[derive(Debug, Default)]
pub struct AccessGroups {
    cache: CacheSlot<AccessGroupMap>,
}

impl CacheClear for AccessGroups {
    // Router._invalidate_access_groups_cache
    fn clear(&self) {
        CacheClear::clear(&self.cache);
    }
}

impl AccessGroups {
    pub fn get_model_access_groups(
        &self,
        lookup: &CatalogLookup<'_>,
        model_name: Option<&str>,
        model_access_group: Option<&str>,
        team_id: Option<&str>,
    ) -> Result<AccessGroupMap, AccessGroupError> {
        let cacheable = model_name.is_none() && model_access_group.is_none() && team_id.is_none();
        if cacheable {
            let cached = self.cache.0.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(stored) = cached.as_ref() {
                return Ok(stored.clone());
            }
        }
        let mut groups = AccessGroupMap::new();
        for row in lookup.get_model_list(model_name, team_id)? {
            add_row_groups(&mut groups, &row, model_access_group)?;
        }
        if cacheable {
            let mut slot = self.cache.0.lock().unwrap_or_else(PoisonError::into_inner);
            *slot = Some(groups.clone());
        }
        Ok(groups)
    }
}

// get_model_access_groups' row loop: every occurrence appends the row's name, and a filter keeps only the exact group.
fn add_row_groups(
    groups: &mut AccessGroupMap,
    row: &DeploymentSpec,
    model_access_group: Option<&str>,
) -> Result<(), AccessGroupError> {
    let declared = match row.model_info.get("access_groups") {
        None | Some(Json::Null) => return Ok(()),
        Some(Json::List(items)) => items,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            return Err(invalid_groups(row));
        }
    };
    for item in declared {
        let Json::Text(group) = item else {
            return Err(invalid_groups(row));
        };
        if model_access_group.is_none_or(|filter| filter == group.as_str()) {
            groups
                .entry(group.clone())
                .or_default()
                .push(row.model_name.clone());
        }
    }
    Ok(())
}

fn invalid_groups(row: &DeploymentSpec) -> AccessGroupError {
    AccessGroupError::InvalidAccessGroups {
        model_name: row.model_name.clone(),
    }
}
