use betterlite_core::agents::AgentResponse;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::rows::RowCodec as _;
use betterlite_db::postgres_store::session::PostgresStore;
use thiserror::Error;

use super::AGENT_SELECT_SQL;
use super::codec::{AgentRowCodec, RowDecodeError};

#[derive(Debug, Error)]
pub enum AgentListError {
    #[error("Error getting agents from DB: {source}")]
    Database {
        #[source]
        source: DbError,
    },
    #[error("Error getting agents from DB: {source}")]
    Decode {
        #[source]
        source: RowDecodeError,
    },
}

pub async fn get_all_agents_from_db(
    store: &PostgresStore,
) -> Result<Vec<AgentResponse>, AgentListError> {
    let client = store
        .session()
        .await
        .map_err(|source| AgentListError::Database { source })?;
    let statement = format!("{AGENT_SELECT_SQL} ORDER BY a.created_at DESC");
    let rows = client
        .query(&statement, &[])
        .await
        .map_err(|source| AgentListError::Database {
            source: backend(source),
        })?;
    let mut agents = Vec::with_capacity(rows.len());
    for row in &rows {
        let stored = AgentRowCodec
            .decode(row)
            .map_err(|source| AgentListError::Decode { source })?;
        if let Some(stored) = stored {
            agents.push(stored.agent);
        }
    }
    Ok(agents)
}

mod tests;
