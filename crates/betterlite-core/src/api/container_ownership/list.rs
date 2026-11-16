use std::collections::BTreeSet;

use betterlite_auth::auth_identity::records::Principal;

use super::cache::allowed_ids_cache_key;
use super::errors::{ContainerListError, ContainerOwnershipError};
use super::repository::ContainerObjectTable;
use super::{
    ContainerCaller, ContainerListPage, ContainerListSource, ContainerOwnership,
    DEFAULT_CONTAINER_LIST_LIMIT, OWNED_CONTAINER_LIST_MAX_PAGES, OWNED_CONTAINER_LIST_PAGE_SIZE,
    container_model_object_id, decode_for_ownership, owner_scope_strings,
};
use crate::api::response_id_traversal::record::Record;

pub async fn list_owned_containers<Rows, Source>(
    ownership: &ContainerOwnership<Rows>,
    source: &Source,
    after: Option<&str>,
    limit: Option<i64>,
    caller: ContainerCaller<'_>,
) -> Result<ContainerListPage, ContainerListError<Source::Failure>>
where
    Rows: ContainerObjectTable,
    Source: ContainerListSource + Sync,
{
    let allowed = allowed_container_ids(ownership, caller.principal).await?;
    let requested = limit.unwrap_or(DEFAULT_CONTAINER_LIST_LIMIT);
    let Ok(page_limit) = usize::try_from(requested) else {
        return Err(ContainerListError::InvalidLimit { limit: requested });
    };
    let needed = page_limit.saturating_add(1);
    let (last_page, owned) =
        collect_owned(source, after, needed, &allowed, caller.custom_llm_provider)
            .await
            .map_err(ContainerListError::Fetch)?;
    Ok(list_page(last_page, owned, page_limit))
}

// _with_container_list_page: the caller's page keeps the upstream keys and carries its own metadata
fn list_page(
    last_page: ContainerListPage,
    owned: Vec<Record>,
    page_limit: usize,
) -> ContainerListPage {
    let has_more = owned.len() > page_limit || last_page.has_more;
    let data: Vec<Record> = owned.into_iter().take(page_limit).collect();
    let first_id = page_id(data.first());
    let last_id = page_id(data.last());
    ContainerListPage {
        data,
        first_id,
        last_id,
        has_more,
        extra: last_page.extra,
    }
}

fn page_id(item: Option<&Record>) -> Option<String> {
    item.and_then(|entry| entry.field("id"))
        .and_then(Record::as_text)
        .map(str::to_owned)
}

// _get_allowed_container_ids: an empty scope set owns nothing, and an identical sorted set shares the entry
pub(crate) async fn allowed_container_ids<Rows>(
    ownership: &ContainerOwnership<Rows>,
    principal: Option<&Principal>,
) -> Result<BTreeSet<String>, ContainerOwnershipError>
where
    Rows: ContainerObjectTable,
{
    let scopes = owner_scope_strings(principal);
    if scopes.is_empty() {
        return Ok(BTreeSet::new());
    }
    let cache_key = allowed_ids_cache_key(&scopes)?;
    if let Some(cached) = ownership.caches.allowed(&cache_key)? {
        return Ok(cached.into_iter().collect());
    }
    if !ownership.database_ready()? {
        return Ok(BTreeSet::new());
    }
    let rows = ownership.objects.find_owned(scopes).await?;
    ownership.caches.store_allowed(&cache_key, &rows)?;
    Ok(rows.into_iter().collect())
}

async fn collect_owned<Source>(
    source: &Source,
    after: Option<&str>,
    needed: usize,
    allowed: &BTreeSet<String>,
    custom_llm_provider: &str,
) -> Result<(ContainerListPage, Vec<Record>), Source::Failure>
where
    Source: ContainerListSource + Sync,
{
    let mut owned: Vec<Record> = Vec::new();
    let mut cursor = after.map(str::to_owned);
    let mut pages_left = OWNED_CONTAINER_LIST_MAX_PAGES;
    loop {
        let page = source
            .fetch_page(cursor.as_deref(), OWNED_CONTAINER_LIST_PAGE_SIZE)
            .await?;
        let page_owned: Vec<Record> = page
            .data
            .iter()
            .filter(|item| is_owned_container(item, allowed, custom_llm_provider))
            .cloned()
            .collect();
        let owned_count = owned.len().saturating_add(page_owned.len());
        let enough = owned_count >= needed;
        owned.extend(page_owned);
        let Some(last_id) = page_last_id(&page) else {
            return Ok((page, owned));
        };
        if enough || pages_left <= 1 || !page.has_more {
            return Ok((page, owned));
        }
        cursor = Some(last_id);
        pages_left = pages_left.saturating_sub(1);
    }
}

fn page_last_id(page: &ContainerListPage) -> Option<String> {
    page.data
        .last()
        .and_then(|item| item.field("id"))
        .and_then(Record::as_text)
        .map(str::to_owned)
}

fn is_owned_container(
    item: &Record,
    allowed: &BTreeSet<String>,
    custom_llm_provider: &str,
) -> bool {
    let Some(container_id) = item.field("id").and_then(Record::as_text) else {
        return false;
    };
    let identity = decode_for_ownership(container_id, custom_llm_provider);
    allowed.contains(&container_model_object_id(
        &identity.original_container_id,
        &identity.custom_llm_provider,
    ))
}
