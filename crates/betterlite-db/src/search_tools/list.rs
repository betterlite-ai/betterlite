use std::error::Error as _;

use crate::auxiliary_repository::repository::TableError;
use crate::database_faults::native::is_native_transport_error;
use crate::database_faults::reconnect::{DbReconnect, RetryPolicy, call_with_db_reconnect_retry};
use crate::database_faults::{DbError, ErrorType};

use super::convert::tool::SearchTool;
use super::error::StoreError;
use super::registry::SearchToolRegistry;
use super::table_client::{CreatedAtOrder, SearchToolTableClient};

mod tests;

const RECONNECT_REASON: &str = "get_all_search_tools_from_db_lookup_failure";

// The retry helper classifies faults by their own error type, so a native transport error maps to Connect
fn read_fault(error: TableError) -> DbError {
    let transport =
        matches!(&error, TableError::Database(native) if is_native_transport_error(native));
    let mut message = error.to_string();
    let mut cause = error.source();
    while let Some(inner) = cause {
        message.push_str(": ");
        message.push_str(&inner.to_string());
        cause = inner.source();
    }
    DbError {
        error_type: if transport {
            ErrorType::Connect
        } else {
            ErrorType::Other
        },
        message,
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}

pub async fn read_all<Table, Client>(
    table: &Table,
    reconnect: Option<&mut Client>,
) -> Result<Vec<SearchTool>, StoreError>
where
    Table: SearchToolTableClient,
    Client: DbReconnect + Send,
{
    let records = call_with_db_reconnect_retry(
        reconnect,
        move || async move {
            table
                .find_many(Some(CreatedAtOrder::Descending))
                .await
                .map_err(read_fault)
        },
        RetryPolicy {
            reason: RECONNECT_REASON,
            retry_safe_error_types: None,
            timeout_seconds: None,
            lock_timeout_seconds: None,
        },
    )
    .await?;
    records
        .iter()
        .map(|record| SearchTool::try_from(SearchToolRegistry::convert_prisma_to_dict(record)))
        .collect()
}
