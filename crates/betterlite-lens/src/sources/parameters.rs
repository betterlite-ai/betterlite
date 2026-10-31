use serde::Serialize;

use crate::models::settings::{MetadataFilter, Scope};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Parameters {
    pub all_teams: i64,
    pub team: String,
    pub key_hash: String,
    pub filter_keys: Vec<String>,
    pub filter_values: Vec<String>,
}

#[must_use]
pub fn parameters(scope: &Scope, filters: &[MetadataFilter]) -> Parameters {
    let mut filter_keys = Vec::with_capacity(filters.len());
    let mut filter_values = Vec::with_capacity(filters.len());
    for filter in filters {
        filter_keys.push(filter.key.clone());
        filter_values.push(filter.value.clone());
    }
    Parameters {
        all_teams: i64::from(scope.all_teams),
        team: scope.team_id.clone(),
        key_hash: scope.api_key_hash.clone(),
        filter_keys,
        filter_values,
    }
}

mod tests;
