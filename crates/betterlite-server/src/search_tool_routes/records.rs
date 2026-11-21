use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::search_tools::convert::SearchToolRow;
use betterlite_db::search_tools::convert::tool::SearchTool;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub(super) struct WriteBody {
    pub(super) search_tool: SearchTool,
}

// dict(prisma_obj): every column of the updated row, with its JSON columns as stored
#[derive(Serialize)]
pub(super) struct UpdatedBody {
    pub(super) search_tool_id: String,
    pub(super) search_tool_name: String,
    pub(super) litellm_params: JsonValue,
    pub(super) search_tool_info: Option<JsonValue>,
    pub(super) created_at: String,
    pub(super) updated_at: String,
}

impl From<SearchToolRow> for UpdatedBody {
    fn from(row: SearchToolRow) -> Self {
        Self {
            search_tool_id: row.search_tool_id,
            search_tool_name: row.search_tool_name,
            litellm_params: row.litellm_params,
            search_tool_info: row.search_tool_info,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Serialize)]
pub(super) struct DeletedBody {
    pub(super) message: String,
    pub(super) search_tool_name: String,
}
