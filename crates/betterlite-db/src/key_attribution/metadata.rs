use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::identity_rows::user::LiteLLMUserTable;

const CLI_SESSION_KEY_PREFIX: &str = "cli-session-";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_exists: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserDetails {
    pub email: Option<String>,
    pub only_team: Option<String>,
}

#[must_use]
pub fn unanimous(first: Option<&str>, last: Option<&str>) -> Option<String> {
    match (first, last) {
        (None, last) => last.map(str::to_owned),
        (first, None) => first.map(str::to_owned),
        (Some(first), Some(last)) if first == last => Some(first.to_owned()),
        (Some(_), Some(_)) => None,
    }
}

#[must_use]
pub fn cli_user_id(api_key: &str) -> Option<&str> {
    api_key
        .strip_prefix(CLI_SESSION_KEY_PREFIX)
        .filter(|rest| !rest.is_empty())
}

#[must_use]
pub fn cli_candidates(keys: &BTreeSet<String>) -> BTreeMap<String, String> {
    keys.iter()
        .filter_map(|key| cli_user_id(key).map(|user_id| (key.clone(), user_id.to_owned())))
        .collect()
}

#[must_use]
pub fn cli_metadata(
    candidates: &BTreeMap<String, String>,
    details: &BTreeMap<String, UserDetails>,
) -> BTreeMap<String, KeyMetadata> {
    candidates
        .iter()
        .filter(|(_, user_id)| details.contains_key(*user_id))
        .map(|(key, user_id)| {
            (
                key.clone(),
                KeyMetadata {
                    key_alias: Some(key.clone()),
                    user_id: Some(user_id.clone()),
                    ..KeyMetadata::default()
                },
            )
        })
        .collect()
}

#[must_use]
pub fn only_team(teams: &[String]) -> Option<String> {
    match teams {
        [team] if !team.is_empty() => Some(team.clone()),
        _ => None,
    }
}

#[must_use]
pub fn user_details(users: &[LiteLLMUserTable]) -> BTreeMap<String, UserDetails> {
    users
        .iter()
        .filter(|user| !user.user_id.is_empty())
        .map(|user| {
            let email = user.user_email.clone().filter(|email| !email.is_empty());
            (
                user.user_id.clone(),
                UserDetails {
                    email,
                    only_team: only_team(&user.teams),
                },
            )
        })
        .collect()
}

#[must_use]
pub fn meta_with_user_details(
    api_key: &str,
    meta: &KeyMetadata,
    details: &BTreeMap<String, UserDetails>,
) -> KeyMetadata {
    let Some(user) = meta.user_id.as_deref().and_then(|id| details.get(id)) else {
        return meta.clone();
    };
    let mut updated = meta.clone();
    let email = meta
        .user_email
        .clone()
        .filter(|email| !email.is_empty())
        .or_else(|| user.email.clone());
    let team = meta
        .team_id
        .clone()
        .filter(|team| !team.is_empty())
        .or_else(|| {
            if cli_user_id(api_key).is_some() {
                user.only_team.clone()
            } else {
                None
            }
        });
    if let Some(email) = email {
        updated.user_email = Some(email);
    }
    if let Some(team) = team {
        updated.team_id = Some(team);
    }
    updated
}

#[must_use]
pub fn user_id_needing_details(api_key: &str, meta: &KeyMetadata) -> Option<String> {
    let user_id = meta
        .user_id
        .as_deref()
        .filter(|user_id| !user_id.is_empty())?;
    let has_email = meta
        .user_email
        .as_deref()
        .is_some_and(|email| !email.is_empty());
    let cli_without_team =
        cli_user_id(api_key).is_some() && meta.team_id.as_deref().is_none_or(str::is_empty);
    if has_email && !cli_without_team {
        return None;
    }
    Some(user_id.to_owned())
}

#[must_use]
pub fn needed_user_ids(recovered: &BTreeMap<String, KeyMetadata>) -> BTreeSet<String> {
    recovered
        .iter()
        .filter_map(|(api_key, meta)| user_id_needing_details(api_key, meta))
        .collect()
}

#[must_use]
pub fn attach_details(
    recovered: &BTreeMap<String, KeyMetadata>,
    details: &BTreeMap<String, UserDetails>,
) -> BTreeMap<String, KeyMetadata> {
    if details.is_empty() {
        return recovered.clone();
    }
    recovered
        .iter()
        .map(|(api_key, meta)| {
            (
                api_key.clone(),
                meta_with_user_details(api_key, meta, details),
            )
        })
        .collect()
}
