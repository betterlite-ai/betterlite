use std::future::Future;

use crate::access_group_read::protocols::error::AccessGroupStoreError;
use crate::access_group_read::protocols::tables::{AccessGroupTx, KeyTable, TeamTable};

use super::error::MemberEffectsError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MembershipTarget {
    Team,
    Key,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MembershipChange {
    Add,
    Remove,
}

// a missing row is skipped while a present row may still carry a null array
enum FoundRow {
    Missing,
    Present(Option<Vec<String>>),
}

trait MembershipRows: Send + Sync {
    fn find_row(
        &self,
        id: &str,
    ) -> impl Future<Output = Result<FoundRow, AccessGroupStoreError>> + Send;

    fn write_groups(
        &self,
        id: &str,
        groups: &[String],
    ) -> impl Future<Output = Result<(), AccessGroupStoreError>> + Send;
}

struct TeamMemberRows<T: TeamTable>(T);

struct KeyMemberRows<T: KeyTable>(T);

impl<T: TeamTable> MembershipRows for TeamMemberRows<T> {
    async fn find_row(&self, id: &str) -> Result<FoundRow, AccessGroupStoreError> {
        match self.0.find(id).await? {
            Some(team) => Ok(FoundRow::Present(team.access_group_ids)),
            None => Ok(FoundRow::Missing),
        }
    }

    async fn write_groups(&self, id: &str, groups: &[String]) -> Result<(), AccessGroupStoreError> {
        self.0.set_access_group_ids(id, groups).await
    }
}

impl<T: KeyTable> MembershipRows for KeyMemberRows<T> {
    async fn find_row(&self, id: &str) -> Result<FoundRow, AccessGroupStoreError> {
        match self.0.find(id).await? {
            Some(key) => Ok(FoundRow::Present(key.access_group_ids)),
            None => Ok(FoundRow::Missing),
        }
    }

    async fn write_groups(&self, id: &str, groups: &[String]) -> Result<(), AccessGroupStoreError> {
        self.0.set_access_group_ids(id, groups).await
    }
}

// LiteLLM's four _sync helpers differ only by table and direction, so one walk covers all of them
pub async fn change_membership<Tx>(
    tx: &Tx,
    target: MembershipTarget,
    ids: &[String],
    access_group_id: &str,
    change: MembershipChange,
) -> Result<(), MemberEffectsError>
where
    Tx: AccessGroupTx + ?Sized,
{
    match target {
        MembershipTarget::Team => {
            let rows = TeamMemberRows(tx.litellm_teamtable());
            change_rows(&rows, ids, access_group_id, change).await
        }
        MembershipTarget::Key => {
            let rows = KeyMemberRows(tx.litellm_verificationtoken());
            change_rows(&rows, ids, access_group_id, change).await
        }
    }
}

async fn change_rows<Rows>(
    rows: &Rows,
    ids: &[String],
    access_group_id: &str,
    change: MembershipChange,
) -> Result<(), MemberEffectsError>
where
    Rows: MembershipRows,
{
    for id in ids {
        let FoundRow::Present(existing) = rows.find_row(id).await? else {
            continue;
        };
        let Some(groups) = updated_groups(change, existing, access_group_id) else {
            continue;
        };
        rows.write_groups(id, &groups).await?;
    }
    Ok(())
}

fn updated_groups(
    change: MembershipChange,
    existing: Option<Vec<String>>,
    access_group_id: &str,
) -> Option<Vec<String>> {
    match change {
        MembershipChange::Add => groups_with_added(existing, access_group_id),
        MembershipChange::Remove => {
            let groups = existing?;
            if !groups.iter().any(|id| id.as_str() == access_group_id) {
                return None;
            }
            Some(removed_groups(groups, access_group_id))
        }
    }
}

pub(super) fn groups_with_added(
    existing: Option<Vec<String>>,
    access_group_id: &str,
) -> Option<Vec<String>> {
    let mut groups = existing.unwrap_or_default();
    if groups.iter().any(|id| id.as_str() == access_group_id) {
        return None;
    }
    groups.push(access_group_id.to_owned());
    Some(groups)
}

pub(super) fn removed_groups(groups: Vec<String>, access_group_id: &str) -> Vec<String> {
    groups
        .into_iter()
        .filter(|id| id.as_str() != access_group_id)
        .collect()
}
