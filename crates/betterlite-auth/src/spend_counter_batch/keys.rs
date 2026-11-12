use std::collections::BTreeSet;

use betterlite_db::budget_permission_rows::JsonData;

use crate::auth_cache::keys::CacheKey;
use crate::resolved_key::auth::UserAPIKeyAuth;

#[derive(Clone, Copy, Debug)]
pub enum CounterId<'value> {
    Text(&'value str),
    // litellm's iterator takes object ids: a placeholder that is not a string names no counter
    Unusable,
}

impl<'value> CounterId<'value> {
    #[must_use]
    pub const fn from_text(value: Option<&'value str>) -> Self {
        match value {
            Some(text) => Self::Text(text),
            None => Self::Unusable,
        }
    }

    #[must_use]
    pub fn from_json(value: Option<&'value JsonData>) -> Self {
        match value {
            Some(JsonData::String(text)) => Self::Text(text),
            Some(
                JsonData::Null
                | JsonData::Bool(_)
                | JsonData::Number(_)
                | JsonData::Array(_)
                | JsonData::Object(_),
            )
            | None => Self::Unusable,
        }
    }
}

pub struct EntityCounterIds<'values> {
    pub token: CounterId<'values>,
    pub team_id: CounterId<'values>,
    pub user_id: CounterId<'values>,
    pub org_id: CounterId<'values>,
    pub project_id: CounterId<'values>,
    pub end_user_id: CounterId<'values>,
}

#[must_use]
pub fn entity_counter_keys(ids: &EntityCounterIds<'_>) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    if let CounterId::Text(token) = ids.token {
        keys.insert(format!("spend:key:{token}"));
    }
    if let CounterId::Text(team_id) = ids.team_id {
        keys.insert(format!("spend:team:{team_id}"));
        if let CounterId::Text(user_id) = ids.user_id {
            keys.insert(format!("spend:team_member:{user_id}:{team_id}"));
        }
    }
    if let CounterId::Text(user_id) = ids.user_id {
        keys.insert(format!("spend:user:{user_id}"));
    }
    if let CounterId::Text(end_user_id) = ids.end_user_id {
        keys.insert(format!("spend:end_user:{end_user_id}"));
    }
    if let CounterId::Text(org_id) = ids.org_id {
        keys.insert(format!("spend:org:{org_id}"));
    }
    if let CounterId::Text(project_id) = ids.project_id {
        keys.insert(CacheKey::ProjectSpend(project_id).render());
    }
    keys
}

pub struct PostCallCounterKeys<'values> {
    pub entity: EntityCounterIds<'values>,
    pub tags: &'values [JsonData],
    pub model_access_groups: &'values [JsonData],
}

#[must_use]
pub fn post_call_counter_keys(fields: &PostCallCounterKeys<'_>) -> BTreeSet<String> {
    let mut keys = entity_counter_keys(&fields.entity);
    for tag in fields.tags {
        if let JsonData::String(text) = tag
            && !text.is_empty()
        {
            keys.insert(format!("spend:tag:{text}"));
        }
    }
    for group in fields.model_access_groups {
        if let JsonData::String(text) = group
            && !text.is_empty()
        {
            keys.insert(CacheKey::ModelAccessGroupSpend(text).render());
        }
    }
    keys
}

#[must_use]
pub fn admission_counter_keys(
    auth: &UserAPIKeyAuth,
    end_user_id: Option<&str>,
) -> BTreeSet<String> {
    entity_counter_keys(&EntityCounterIds {
        token: CounterId::from_text(
            auth.view
                .token
                .token
                .as_ref()
                .map(secrecy::ExposeSecret::expose_secret),
        ),
        team_id: CounterId::from_text(auth.view.token.team_id.as_deref()),
        user_id: CounterId::from_text(auth.view.token.user_id.as_deref()),
        org_id: CounterId::from_text(auth.view.token.org_id.as_deref()),
        project_id: CounterId::from_text(auth.view.token.project_id.as_deref()),
        end_user_id: CounterId::from_text(end_user_id),
    })
}
