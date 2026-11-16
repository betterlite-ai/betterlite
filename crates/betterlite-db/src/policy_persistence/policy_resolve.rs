mod tests;

use betterlite_policy::inheritance::resolve_policy_guardrails;
use betterlite_policy::row_adapter::PolicyTableSource;

use super::error::Operation;
use super::policy_list::get_all_policies_from_db;
use super::stored_policies::policy_map;
use super::{PolicyError, PolicyStore};

pub async fn resolve_guardrails_from_db<Client>(
    store: &PolicyStore<'_, Client>,
    policy_name: &str,
) -> Result<Vec<String>, PolicyError<Client>>
where
    Client: PolicyTableSource,
{
    let production = get_all_policies_from_db(store, Some("production"))
        .await
        .map_err(|error| error.within(Operation::ResolveGuardrails).logged())?;
    let policies = policy_map::<Client>(&production, Operation::ResolveGuardrails)?;
    Ok(resolve_policy_guardrails(policy_name, &policies, None).guardrails)
}
