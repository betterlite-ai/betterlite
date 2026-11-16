mod tests;

use betterlite_policy::pipeline::json_value::JsonValue;
use betterlite_policy::records::db::PolicyDBResponse;
use betterlite_policy::row_adapter::{
    PolicyClause, PolicyOrder, PolicySort, PolicyTable as _, PolicyTableSource, policy_db_response,
};

use super::error::Operation;
use super::{PolicyError, PolicyStore};

pub async fn get_all_policies_from_db<Client>(
    store: &PolicyStore<'_, Client>,
    version_status: Option<&str>,
) -> Result<Vec<PolicyDBResponse>, PolicyError<Client>>
where
    Client: PolicyTableSource,
{
    let filter = version_status.map(|status| {
        PolicyClause::from([(
            "version_status".to_owned(),
            JsonValue::String(status.to_owned()),
        )])
    });
    let order = PolicyOrder::from([("created_at".to_owned(), PolicySort::Desc)]);
    let rows = store
        .client
        .policy_table()
        .find_many(filter, Some(order))
        .await
        .map_err(|source| {
            PolicyError::<Client>::Database(vec![Operation::ListPolicies], source).logged()
        })?;
    Ok(rows.iter().map(policy_db_response).collect())
}
