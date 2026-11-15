mod tests;

use std::error::Error;

use crate::pipeline::json_value::JsonValue;
use crate::records::db::PolicyDBResponse;
use crate::row_adapter::{
    PolicyClause, PolicyRow, PolicyTable, PolicyTableSource, policy_db_response,
};

#[derive(Debug, thiserror::Error)]
pub enum PolicyLookupError<Failure>
where
    Failure: Error + Send + Sync + 'static,
{
    #[error("Error getting policy from DB: {source}")]
    Database {
        #[source]
        source: Failure,
    },
}

#[must_use]
pub async fn get_policy_by_id_from_db<Client>(
    policy_id: &str,
    client: &Client,
) -> Result<
    Option<PolicyDBResponse>,
    PolicyLookupError<<Client::Table as PolicyTable<PolicyRow>>::Failure>,
>
where
    Client: PolicyTableSource,
{
    let filter = PolicyClause::from([(
        "policy_id".to_owned(),
        JsonValue::String(policy_id.to_owned()),
    )]);
    match client.policy_table().find_unique(filter).await {
        Ok(None) => Ok(None),
        Ok(Some(row)) => Ok(Some(policy_db_response(&row))),
        Err(source) => {
            tracing::error!("Error getting policy from DB: {source}");
            Err(PolicyLookupError::Database { source })
        }
    }
}
