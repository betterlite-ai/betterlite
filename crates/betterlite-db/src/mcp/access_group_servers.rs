use deadpool_postgres::GenericClient as _;

use crate::postgres_store::errors::backend;

use super::table_access::store::McpTableStore;
use super::table_access::types::McpTableError;

// Prisma's find_many(where={"mcp_access_groups": {"hasSome": access_groups}}) is array overlap, so one shared name matches
const SERVER_IDS_BY_ACCESS_GROUP: &str = "SELECT \"server_id\" FROM \"LiteLLM_MCPServerTable\" WHERE \"mcp_access_groups\" && $1::text[]";

impl McpTableStore<'_> {
    // ast-grep-ignore: no-long-names
    pub async fn server_ids_with_any_access_group(
        &self,
        access_groups: &[String],
        use_writer: bool,
    ) -> Result<Vec<String>, McpTableError> {
        // PrismaTableRepository.table reads writer_db when use_writer is set and db otherwise
        let store = if use_writer {
            self.client.writer
        } else {
            self.reader()
        };
        let client = store.session().await?;
        let groups = access_groups.to_vec();
        let found = client
            .query(SERVER_IDS_BY_ACCESS_GROUP, &[&groups])
            .await
            .map_err(|source| McpTableError::Database(backend(source)))?;
        let mut server_ids = Vec::with_capacity(found.len());
        for row in &found {
            let server_id: String = row
                .try_get(0)
                .map_err(|source| McpTableError::Database(backend(source)))?;
            server_ids.push(server_id);
        }
        Ok(server_ids)
    }
}

mod tests;
