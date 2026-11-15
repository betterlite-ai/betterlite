use chrono::{DateTime, Utc};

use crate::database_faults::reconnect::DbReconnect;

use super::convert::tool::SearchTool;
use super::convert::{SearchToolRow, isoformat};
use super::create;
use super::delete::{self, DeletedSearchTool};
use super::error::SearchToolsError;
use super::get_by_id;
use super::list;
use super::record::SearchToolRecord;
use super::table_client::SearchToolTableClient;
use super::update;

pub struct SearchToolRegistry {
    pub now: fn() -> DateTime<Utc>,
}

// The workspace denies multiple_inherent_impl, so each operation's steps are free functions in its own module
impl SearchToolRegistry {
    #[must_use]
    pub fn convert_prisma_to_dict(prisma_obj: &SearchToolRecord) -> SearchToolRow {
        SearchToolRow {
            search_tool_id: prisma_obj.search_tool_id.clone(),
            search_tool_name: prisma_obj.search_tool_name.clone(),
            litellm_params: prisma_obj.litellm_params.clone(),
            search_tool_info: prisma_obj.search_tool_info.clone(),
            created_at: isoformat(prisma_obj.created_at),
            updated_at: isoformat(prisma_obj.updated_at),
        }
    }

    pub async fn add_search_tool_to_db<Table>(
        &self,
        search_tool: SearchTool,
        table: &Table,
    ) -> Result<SearchTool, SearchToolsError>
    where
        Table: SearchToolTableClient,
    {
        let record = create::persist((self.now)(), &search_tool, table).await?;
        let mut stored = search_tool;
        stored.search_tool_id = Some(record.search_tool_id);
        stored.created_at = Some(isoformat(record.created_at));
        stored.updated_at = Some(isoformat(record.updated_at));
        Ok(stored)
    }

    pub async fn delete_search_tool_from_db<Table>(
        &self,
        search_tool_id: &str,
        table: &Table,
    ) -> Result<DeletedSearchTool, SearchToolsError>
    where
        Table: SearchToolTableClient,
    {
        let existing = delete::lookup::existing(search_tool_id, table).await?;
        let message = format!("Search tool {search_tool_id} deleted successfully");
        delete::remove(search_tool_id, table).await?;
        Ok(DeletedSearchTool {
            message,
            search_tool_name: existing.search_tool_name,
        })
    }

    pub async fn update_search_tool_in_db<Table>(
        &self,
        search_tool_id: &str,
        search_tool: &SearchTool,
        table: &Table,
    ) -> Result<SearchToolRow, SearchToolsError>
    where
        Table: SearchToolTableClient,
    {
        let updated_at = (self.now)();
        let record = update::replace(search_tool_id, search_tool, updated_at, table).await?;
        Ok(Self::convert_prisma_to_dict(&record))
    }

    pub async fn get_all_search_tools_from_db<Table, Client>(
        table: &Table,
        reconnect: Option<&mut Client>,
    ) -> Result<Vec<SearchTool>, SearchToolsError>
    where
        Table: SearchToolTableClient,
        Client: DbReconnect + Send,
    {
        let records = list::read_all(table, reconnect).await?;
        let mut tools = Vec::with_capacity(records.len());
        for record in &records {
            let row = Self::convert_prisma_to_dict(record);
            let tool = SearchTool::try_from(row)
                .map_err(|error| SearchToolsError::List(error).logged())?;
            tools.push(tool);
        }
        Ok(tools)
    }

    pub async fn get_search_tool_by_id_from_db<Table>(
        &self,
        search_tool_id: &str,
        table: &Table,
    ) -> Result<Option<SearchTool>, SearchToolsError>
    where
        Table: SearchToolTableClient,
    {
        get_by_id::lookup(search_tool_id, table)
            .await?
            .map(|record| Self::convert_prisma_to_dict(&record))
            .map(SearchTool::try_from)
            .transpose()
            .map_err(|error| SearchToolsError::Get(error).logged())
    }
}
