mod tests;

use std::error::Error;

use crate::pipeline::json_value::JsonValue;
use crate::records::db::PolicyDBResponse;
use crate::row_adapter::{PolicyClause, PolicyTable, PolicyTableSource, policy_db_response};

#[derive(Debug, thiserror::Error)]
pub enum PolicyLookupError {
    #[error("Error getting policy from DB: {source}")]
    Database {
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
}

#[must_use]
pub async fn get_policy_by_id_from_db<Client>(
    policy_id: &str,
    client: &Client,
) -> Result<Option<PolicyDBResponse>, PolicyLookupError>
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
        Err(error) => {
            tracing::error!("Error getting policy from DB: {error}");
            Err(PolicyLookupError::Database {
                source: Box::new(error),
            })
        }
    }
}
