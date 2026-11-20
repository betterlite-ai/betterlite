use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::errors::DbError;

use crate::key_query_plan::KeyQueryPlanError;
use crate::key_query_plan::plan::{KeyListQuery, KeyQueryPlan, KeyTable, build_plan};
use crate::key_query_plan::response::{
    KeyListEntry, KeyListResponseObject, KeyRow, attach_user_expansion, entry_for_row,
};
use crate::key_query_plan::source::KeyListSource;

pub async fn list_keys(
    client: Option<PrismaClient<'_>>,
    query: &KeyListQuery,
) -> Result<KeyListResponseObject, KeyQueryPlanError> {
    let Some(prisma) = client else {
        return Err(KeyQueryPlanError::Database(DbError::NoDbConnected));
    };
    list_keys_with(&prisma, query).await
}

// page first, then count, with no surrounding transaction, so a concurrent write can show up in total_count
pub async fn list_keys_with<S>(
    source: &S,
    query: &KeyListQuery,
) -> Result<KeyListResponseObject, KeyQueryPlanError>
where
    S: KeyListSource + Sync,
{
    let plan = build_plan(query)?;
    tracing::debug!(conditions = ?plan.where_clause, "Filter conditions");
    tracing::debug!(skip = plan.skip, take = plan.take, "Pagination");
    let rows = source.fetch_page(&plan).await?;
    tracing::debug!(count = rows.len(), "Fetched keys");
    let total_count = source.count(&plan).await?;
    tracing::debug!(total_count, "Total count of keys");
    let user_map = load_user_map(source, &rows, query).await?;
    let keys = render_rows(source, &plan, rows, &user_map, query).await?;
    let padded = total_count.checked_add(plan.take.saturating_sub(1));
    let divided = padded.and_then(|sum| sum.checked_div(plan.take));
    Ok(KeyListResponseObject {
        keys,
        total_count: Some(total_count),
        current_page: Some(query.page),
        total_pages: Some(divided.unwrap_or(i64::MAX)),
    })
}

async fn load_user_map<S>(
    source: &S,
    rows: &[KeyRow],
    query: &KeyListQuery,
) -> Result<BTreeMap<String, LiteLLMUserTable>, KeyQueryPlanError>
where
    S: KeyListSource,
{
    if !query.expand.iter().any(|name| name == "user") {
        return Ok(BTreeMap::new());
    }
    let mut ids = BTreeSet::new();
    for row in rows {
        for field in ["user_id", "created_by"] {
            if let Some(JsonData::String(text)) = row.get(field)
                && !text.is_empty()
            {
                ids.insert(text.clone());
            }
        }
    }
    if ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let users = source.find_users(ids.into_iter().collect()).await?;
    Ok(users
        .into_iter()
        .map(|user| (user.user_id.clone(), user))
        .collect())
}

async fn render_rows<S>(
    source: &S,
    plan: &KeyQueryPlan,
    rows: Vec<KeyRow>,
    user_map: &BTreeMap<String, LiteLLMUserTable>,
    query: &KeyListQuery,
) -> Result<Vec<KeyListEntry>, KeyQueryPlanError>
where
    S: KeyListSource,
{
    let expand_user = query.expand.iter().any(|name| name == "user");
    let full_object = query.return_full_object || expand_user;
    let mut keys = Vec::with_capacity(rows.len());
    for mut row in rows {
        if matches!(plan.table, KeyTable::Live) {
            source.attach_object_permission(&mut row).await?;
        }
        if expand_user {
            attach_user_expansion(&mut row, user_map)?;
        }
        keys.push(entry_for_row(row, plan.table, full_object)?);
    }
    Ok(keys)
}
