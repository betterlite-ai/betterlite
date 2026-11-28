use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::member_removal_identity::identity::MemberIdentity;
use betterlite_db::member_removal_identity::matching::{
    addresses_member, addresses_user, with_row_email,
};
use betterlite_db::team_rows::member::Member;

use super::{RemovalTables, Selection};
use crate::bulk_team_removal::error::BulkRemovalError;

pub(super) async fn select_members(
    tables: &dyn RemovalTables,
    team_id: &str,
    members: &[MemberIdentity],
    roster: Vec<Member>,
) -> Result<Selection, BulkRemovalError> {
    let (requests, requested_rows) = backfilled_requests(tables, team_id, members).await?;
    let mut removed = Vec::new();
    let mut kept = Vec::new();
    for member in roster {
        if requests
            .iter()
            .any(|request| addresses_member(&member, request))
        {
            removed.push(member);
        } else {
            kept.push(member);
        }
    }
    let removed_ids = member_ids(&removed);
    let removed_rows = unfetched_rows(tables, &removed_ids, &requested_rows).await?;
    let stale_rows: Vec<LiteLLMUserTable> = requested_rows
        .iter()
        .chain(removed_rows.iter())
        .filter(|user| user.teams.iter().any(|team| team == team_id))
        .cloned()
        .collect();
    let cleanup_ids = merged_cleanup_ids(&removed_ids, &stale_rows);
    let matched = matched_indexes(&requests, &removed, &stale_rows);
    Ok(Selection {
        removed,
        kept,
        stale_rows,
        cleanup_ids,
        matched,
    })
}

async fn backfilled_requests(
    tables: &dyn RemovalTables,
    team_id: &str,
    members: &[MemberIdentity],
) -> Result<(Vec<MemberIdentity>, Vec<LiteLLMUserTable>), BulkRemovalError> {
    let (user_ids, user_emails) = requested_identities(members);
    let requested_rows = if user_ids.is_empty() && user_emails.is_empty() {
        Vec::new()
    } else {
        tables.users(&user_ids, &user_emails).await?
    };
    let email_of = row_emails(team_id, &requested_rows);
    let requests = members
        .iter()
        .cloned()
        .map(|request| with_row_email(request, &email_of))
        .collect();
    Ok((requests, requested_rows))
}

fn requested_identities(members: &[MemberIdentity]) -> (Vec<String>, Vec<String>) {
    let mut user_ids = BTreeSet::new();
    let mut user_emails = BTreeSet::new();
    for member in members {
        match &member.user_id {
            Some(user_id) => {
                user_ids.insert(user_id.clone());
            }
            None => {
                let email = member
                    .user_email
                    .as_deref()
                    .filter(|requested| !requested.is_empty());
                if let Some(requested) = email {
                    user_emails.insert(requested.to_owned());
                }
            }
        }
    }
    (
        user_ids.into_iter().collect(),
        user_emails.into_iter().collect(),
    )
}

fn row_emails(team_id: &str, rows: &[LiteLLMUserTable]) -> BTreeMap<String, String> {
    rows.iter()
        .filter(|user| user.teams.iter().any(|team| team == team_id))
        .filter_map(|user| {
            user.user_email
                .clone()
                .map(|email| (user.user_id.clone(), email))
        })
        .collect()
}

fn member_ids(members: &[Member]) -> BTreeSet<String> {
    members
        .iter()
        .filter_map(|member| member.base.user_id.clone())
        .collect()
}

async fn unfetched_rows(
    tables: &dyn RemovalTables,
    removed_ids: &BTreeSet<String>,
    requested_rows: &[LiteLLMUserTable],
) -> Result<Vec<LiteLLMUserTable>, BulkRemovalError> {
    let loaded: BTreeSet<&str> = requested_rows
        .iter()
        .map(|user| user.user_id.as_str())
        .collect();
    let unfetched: Vec<String> = removed_ids
        .iter()
        .filter(|user_id| !loaded.contains(user_id.as_str()))
        .cloned()
        .collect();
    if unfetched.is_empty() {
        return Ok(Vec::new());
    }
    tables.users(&unfetched, &[]).await
}

fn merged_cleanup_ids(
    removed_ids: &BTreeSet<String>,
    stale_rows: &[LiteLLMUserTable],
) -> Vec<String> {
    let mut ids = removed_ids.clone();
    ids.extend(stale_rows.iter().map(|user| user.user_id.clone()));
    ids.into_iter().collect()
}

fn matched_indexes(
    requests: &[MemberIdentity],
    removed: &[Member],
    stale_rows: &[LiteLLMUserTable],
) -> BTreeSet<usize> {
    requests
        .iter()
        .enumerate()
        .filter(|(_, request)| {
            removed
                .iter()
                .any(|member| addresses_member(member, request))
                || stale_rows.iter().any(|user| addresses_user(user, request))
        })
        .map(|(index, _)| index)
        .collect()
}
