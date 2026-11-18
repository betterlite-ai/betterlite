use std::collections::BTreeSet;

use betterlite_db::budget_permission_rows::JsonData;

use crate::auth_cache::keys::CacheKey;
use crate::resolved_key::auth::UserAPIKeyAuth;

// a placeholder id that is not a string names no counter, an empty string still does
#[derive(Clone, Copy, Debug, Default)]
pub struct CounterEntities<'ids> {
    pub key_token: Option<&'ids str>,
    pub team: Option<&'ids str>,
    pub user: Option<&'ids str>,
    pub end_user: Option<&'ids str>,
    pub org: Option<&'ids str>,
    pub project: Option<&'ids str>,
}

#[must_use]
pub fn entity_counter_keys(entities: &CounterEntities<'_>) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    let named = [
        ("spend:key:", entities.key_token),
        ("spend:team:", entities.team),
        ("spend:user:", entities.user),
        ("spend:end_user:", entities.end_user),
        ("spend:org:", entities.org),
    ];
    for (prefix, id) in named {
        if let Some(id) = id {
            keys.insert(format!("{prefix}{id}"));
        }
    }
    if let (Some(team), Some(user)) = (entities.team, entities.user) {
        keys.insert(format!("spend:team_member:{user}:{team}"));
    }
    if let Some(project) = entities.project {
        keys.insert(CacheKey::ProjectSpend(project).render());
    }
    keys
}

pub struct PostCallCounters<'values> {
    pub entities: CounterEntities<'values>,
    pub tags: &'values [JsonData],
    pub access_groups: &'values [JsonData],
}

#[must_use]
pub fn post_call_counter_keys(counters: &PostCallCounters<'_>) -> BTreeSet<String> {
    let mut keys = entity_counter_keys(&counters.entities);
    let tags = counters.tags.iter().filter_map(|tag| match tag {
        JsonData::String(text) if !text.is_empty() => Some(format!("spend:tag:{text}")),
        _ => None,
    });
    keys.extend(tags);
    let groups = counters
        .access_groups
        .iter()
        .filter_map(|group| match group {
            JsonData::String(text) if !text.is_empty() => {
                Some(CacheKey::ModelAccessGroupSpend(text).render())
            }
            _ => None,
        });
    keys.extend(groups);
    keys
}

#[must_use]
pub fn admission_counter_keys(
    auth: &UserAPIKeyAuth,
    end_user_id: Option<&str>,
) -> BTreeSet<String> {
    let token = &auth.view.token;
    entity_counter_keys(&CounterEntities {
        key_token: token
            .token
            .as_ref()
            .map(secrecy::ExposeSecret::expose_secret),
        team: token.team_id.as_deref(),
        user: token.user_id.as_deref(),
        end_user: end_user_id,
        org: token.org_id.as_deref(),
        project: token.project_id.as_deref(),
    })
}
