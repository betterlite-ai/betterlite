use std::iter;

use betterlite_db::auxiliary_repository::bindings::EndUserTable;
use betterlite_db::auxiliary_repository::repository::TableBinding as _;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// LiteLLM's _RESTRICTED_COLUMNS: a non-null value in any of these makes a row restrict something
pub const RESTRICTED_COLUMNS: [&str; 4] = [
    "budget_id",
    "allowed_model_region",
    "default_model",
    "object_permission_id",
];

// LiteLLM's _column_is_set: ``column IS NOT NULL`` as the filter dict Prisma's builder accepts
#[must_use]
pub fn column_is_set(column: &str) -> Json {
    Json::Object(IndexMap::from([(
        column.to_owned(),
        Json::Object(IndexMap::from([("not".to_owned(), Json::Null)])),
    )]))
}

// LiteLLM's _restricted_end_user_where: blocked=true plus one IS NOT NULL alternative per column
#[must_use]
pub fn restricted_end_user_where() -> Json {
    let blocked = Json::Object(IndexMap::from([("blocked".to_owned(), Json::Bool(true))]));
    let columns = RESTRICTED_COLUMNS.map(column_is_set);
    let alternatives = iter::once(blocked).chain(columns).collect();
    Json::Object(IndexMap::from([(
        "OR".to_owned(),
        Json::List(alternatives),
    )]))
}

// the same predicate as the SQL betterlite runs; blocked restricts on TRUE, every column on IS NOT NULL
#[must_use]
pub fn restricted_end_user_sql() -> String {
    let mut alternatives = String::from("\"blocked\" = TRUE");
    for column in RESTRICTED_COLUMNS {
        alternatives.push_str(" OR \"");
        alternatives.push_str(column);
        alternatives.push_str("\" IS NOT NULL");
    }
    format!(
        "SELECT \"user_id\" FROM \"{}\" WHERE {alternatives} LIMIT $1",
        EndUserTable::TABLE
    )
}
