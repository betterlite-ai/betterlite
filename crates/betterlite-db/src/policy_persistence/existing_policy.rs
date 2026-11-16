use betterlite_policy::pipeline::json_value::JsonValue;
use betterlite_policy::row_adapter::{
    PolicyClause, PolicyRow, PolicyTable as _, PolicyTableSource,
};

use super::error::Operation;
use super::{PolicyError, PolicyStore};

pub(super) async fn existing_policy<Client>(
    store: &PolicyStore<'_, Client>,
    policy_id: &str,
    operation: Operation,
) -> Result<(PolicyClause, PolicyRow), PolicyError<Client>>
where
    Client: PolicyTableSource,
{
    let chain = || vec![operation];
    let filter = PolicyClause::from([(
        "policy_id".to_owned(),
        JsonValue::String(policy_id.to_owned()),
    )]);
    let found = store
        .client
        .policy_table()
        .find_unique(filter.clone())
        .await
        .map_err(|source| PolicyError::<Client>::Database(chain(), source).logged())?;
    let Some(policy) = found else {
        return Err(PolicyError::<Client>::PolicyNotFound(chain(), policy_id.to_owned()).logged());
    };
    Ok((filter, policy))
}
