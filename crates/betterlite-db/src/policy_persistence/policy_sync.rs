mod tests;

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_policy::pipeline::json_value::JsonValue;
use betterlite_policy::registry::{PolicyRegistry, PolicySource};
use betterlite_policy::row_adapter::{
    PolicyClause, PolicyOrder, PolicySort, PolicyTable as _, PolicyTableSource, policy_db_response,
};
use betterlite_policy::values::Policy;

use super::error::Operation;
use super::policy_list::get_all_policies_from_db;
use super::stored_policies::{parse_stored_policy, policy_map};
use super::{PolicyError, PolicyStore};

pub async fn sync_policies_from_db<Client>(
    store: &PolicyStore<'_, Client>,
) -> Result<(), PolicyError<Client>>
where
    Client: PolicyTableSource,
{
    let production = get_all_policies_from_db(store, Some("production"))
        .await
        .map_err(|error| error.within(Operation::SyncPolicies).logged())?;
    let db_policies = policy_map::<Client>(&production, Operation::SyncPolicies)?;
    publish_production(store.registry, db_policies);
    let by_id_count = load_versions_by_id(store).await?;
    let config_count = {
        let mut registry = store
            .registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        registry.initialized = true;
        registry.config_policies.len()
    };
    tracing::info!(
        "Synced {} production policies and {by_id_count} draft/published (by ID) from DB to in-memory registry ({config_count} config-defined policies preserved)",
        production.len()
    );
    Ok(())
}

fn publish_production(shared: &Mutex<PolicyRegistry>, db_policies: BTreeMap<String, Policy>) {
    let mut registry = shared.lock().unwrap_or_else(PoisonError::into_inner);
    for policy_name in db_policies
        .keys()
        .filter(|name| registry.config_policies.contains_key(*name))
    {
        tracing::warn!(
            "Policy '{policy_name}' is defined in both config.yaml and the DB; the DB version takes precedence"
        );
    }
    let config_sources = registry
        .config_policies
        .keys()
        .map(|name| (name.clone(), PolicySource::Config));
    let db_sources = db_policies
        .keys()
        .map(|name| (name.clone(), PolicySource::Db));
    registry.sources = config_sources.chain(db_sources).collect();
    let mut policies = registry.config_policies.clone();
    policies.extend(db_policies);
    registry.policies = policies;
    registry.policies_by_id = BTreeMap::new();
}

async fn load_versions_by_id<Client>(
    store: &PolicyStore<'_, Client>,
) -> Result<usize, PolicyError<Client>>
where
    Client: PolicyTableSource,
{
    let statuses = vec![
        JsonValue::String("draft".to_owned()),
        JsonValue::String("published".to_owned()),
    ];
    let wanted = JsonValue::Object(BTreeMap::from([(
        "in".to_owned(),
        JsonValue::Array(statuses),
    )]));
    let filter = PolicyClause::from([("version_status".to_owned(), wanted)]);
    let order = PolicyOrder::from([("created_at".to_owned(), PolicySort::Desc)]);
    let table = store.client.policy_table();
    let rows = table
        .find_many(Some(filter), Some(order))
        .await
        .map_err(|source| {
            PolicyError::<Client>::Database(vec![Operation::SyncPolicies], source).logged()
        })?;
    for row in &rows {
        let policy =
            parse_stored_policy::<Client>(&policy_db_response(row), Operation::SyncPolicies)?;
        let mut registry = store
            .registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        registry
            .policies_by_id
            .insert(row.policy_id.clone(), (row.policy_name.clone(), policy));
    }
    Ok(rows.len())
}
