use std::collections::BTreeSet;
use std::future::Future;

use betterlite_auth::auth_invalidation::error::InvalidationError;
use betterlite_db::postgres_store::errors::DbError;
use thiserror::Error;

use super::cache::AccessGroupCache;

pub mod postgres;

#[derive(Debug, Error)]
pub enum KeySyncError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Cache(#[from] InvalidationError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovedGroupRow {
    pub access_group_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyMembershipStatement {
    Attach,
    Detach,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAccessGroupUpdate<'request> {
    Absent,
    Cleared,
    Groups(&'request [String]),
}

pub struct KeyRegeneration<'request> {
    pub previous_key_token: &'request str,
    pub new_key_token: &'request str,
    pub update: Option<KeyAccessGroupUpdate<'request>>,
    pub existing_access_group_ids: Option<&'request [String]>,
}

pub trait KeyMembershipDatabase: Send + Sync {
    fn move_key_membership(
        &self,
        statement: KeyMembershipStatement,
        key_token: &str,
        access_group_ids: &[String],
    ) -> impl Future<Output = Result<Vec<MovedGroupRow>, KeySyncError>> + Send;

    fn repoint_key_membership(
        &self,
        previous_key_token: &str,
        new_key_token: &str,
    ) -> impl Future<Output = Result<Vec<MovedGroupRow>, KeySyncError>> + Send;
}

fn group_set(access_group_ids: Option<&[String]>) -> BTreeSet<String> {
    let Some(groups) = access_group_ids else {
        return BTreeSet::new();
    };
    groups.iter().cloned().collect()
}

async fn invalidate_moved_groups<Cache>(
    cache: &Cache,
    moved: Vec<MovedGroupRow>,
) -> Result<(), KeySyncError>
where
    Cache: AccessGroupCache + ?Sized,
{
    for row in moved {
        cache.delete_access_group(&row.access_group_id).await?;
    }
    Ok(())
}

async fn write_key_membership<Database, Cache>(
    database: &Database,
    cache: &Cache,
    statement: KeyMembershipStatement,
    access_group_ids: &BTreeSet<String>,
    key_token: &str,
) -> Result<(), KeySyncError>
where
    Database: KeyMembershipDatabase + ?Sized,
    Cache: AccessGroupCache + ?Sized,
{
    if access_group_ids.is_empty() {
        return Ok(());
    }
    let sorted: Vec<String> = access_group_ids.iter().cloned().collect();
    let moved = database
        .move_key_membership(statement, key_token, &sorted)
        .await?;
    invalidate_moved_groups(cache, moved).await
}

pub async fn sync_key_access_groups<Database, Cache>(
    database: &Database,
    cache: &Cache,
    key_token: &str,
    previous_access_group_ids: Option<&[String]>,
    updated_access_group_ids: Option<&[String]>,
) -> Result<(), KeySyncError>
where
    Database: KeyMembershipDatabase + ?Sized,
    Cache: AccessGroupCache + ?Sized,
{
    let previous = group_set(previous_access_group_ids);
    let updated = group_set(updated_access_group_ids);
    let attach: BTreeSet<String> = updated.difference(&previous).cloned().collect();
    let detach: BTreeSet<String> = previous.difference(&updated).cloned().collect();
    write_key_membership(
        database,
        cache,
        KeyMembershipStatement::Attach,
        &attach,
        key_token,
    )
    .await?;
    write_key_membership(
        database,
        cache,
        KeyMembershipStatement::Detach,
        &detach,
        key_token,
    )
    .await
}

pub async fn sync_key_update_access_groups<Database, Cache>(
    database: &Database,
    cache: &Cache,
    key_token: &str,
    update: KeyAccessGroupUpdate<'_>,
    existing_access_group_ids: Option<&[String]>,
) -> Result<(), KeySyncError>
where
    Database: KeyMembershipDatabase + ?Sized,
    Cache: AccessGroupCache + ?Sized,
{
    let updated = match update {
        KeyAccessGroupUpdate::Absent => return Ok(()),
        KeyAccessGroupUpdate::Cleared => None,
        KeyAccessGroupUpdate::Groups(group_ids) => Some(group_ids),
    };
    sync_key_access_groups(
        database,
        cache,
        key_token,
        existing_access_group_ids,
        updated,
    )
    .await
}

pub async fn sync_key_regeneration<Database, Cache>(
    database: &Database,
    cache: &Cache,
    regeneration: KeyRegeneration<'_>,
) -> Result<(), KeySyncError>
where
    Database: KeyMembershipDatabase + ?Sized,
    Cache: AccessGroupCache + ?Sized,
{
    let moved = database
        .repoint_key_membership(regeneration.previous_key_token, regeneration.new_key_token)
        .await?;
    invalidate_moved_groups(cache, moved).await?;
    let Some(update) = regeneration.update else {
        return Ok(());
    };
    sync_key_update_access_groups(
        database,
        cache,
        regeneration.new_key_token,
        update,
        regeneration.existing_access_group_ids,
    )
    .await
}
