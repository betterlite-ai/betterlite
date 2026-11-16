mod tests;

use std::sync::PoisonError;

use betterlite_policy::registry::PolicySource;
use betterlite_policy::row_adapter::{PolicyTable as _, PolicyTableSource};

use super::error::Operation;
use super::existing_policy::existing_policy;
use super::{Deletion, PolicyError, PolicyStore};

const CONFIG_RESTORED: &str =
    "Production version was deleted. The config-defined policy with the same name is active again.";

const NONE_PROMOTED: &str = "Production version was deleted. No other version was promoted. Promote another version to production if this policy should remain active.";

pub async fn delete_policy_from_db<Client>(
    store: &PolicyStore<'_, Client>,
    policy_id: &str,
) -> Result<Deletion, PolicyError<Client>>
where
    Client: PolicyTableSource,
{
    let (filter, policy) = existing_policy(store, policy_id, Operation::DeletePolicy).await?;
    let table = store.client.policy_table();
    table.delete(filter).await.map_err(|source| {
        PolicyError::<Client>::Database(vec![Operation::DeletePolicy], source).logged()
    })?;
    let message = format!("Policy {policy_id} deleted successfully");
    if policy.version_status.as_deref().unwrap_or("production") != "production" {
        return Ok(Deletion {
            message,
            warning: None,
        });
    }
    let config_restored = {
        let mut registry = store
            .registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        registry.remove_policy(&policy.policy_name);
        registry.sources.get(&policy.policy_name) == Some(&PolicySource::Config)
    };
    let warning = if config_restored {
        CONFIG_RESTORED
    } else {
        NONE_PROMOTED
    };
    Ok(Deletion {
        message,
        warning: Some(warning.to_owned()),
    })
}
