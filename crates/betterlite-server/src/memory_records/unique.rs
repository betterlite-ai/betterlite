use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::postgres_store::errors::is_unique_violation as database_unique_violation;

const P2002: &str = "P2002";
const FALLBACKS: [&str; 4] = [P2002, "Unique", "unique", "UniqueViolation"];

// memory_endpoints.py _is_unique_violation: the typed Prisma code first, then the message text
#[must_use]
pub fn is_unique_violation(code: Option<&str>, message: &str) -> bool {
    if code == Some(P2002) {
        return true;
    }
    FALLBACKS.iter().any(|marker| message.contains(marker))
}

// the table-layer form: betterlite's DbError carries the SQLSTATE, so that check runs before the text
#[must_use]
pub fn is_table_unique_violation(error: &TableError) -> bool {
    let typed = match error {
        TableError::Database(source) => database_unique_violation(source),
        TableError::Batch(_)
        | TableError::Row(_)
        | TableError::MissingRow
        | TableError::RowCount { .. }
        | TableError::Publish { .. } => false,
    };
    if typed {
        return true;
    }
    is_unique_violation(None, &error.to_string())
}
