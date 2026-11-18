use std::collections::BTreeMap;
use std::str::FromStr as _;

use betterlite_auth::auth_cache::keys::CacheKey;
use betterlite_auth::auth_cache::payload;
use betterlite_db::budget_reset_policy::cache_invalidation::GLOBAL_PROXY_SPEND_CACHE_KEY;
use betterlite_db::identity_rows::end_user::LiteLLMEndUserTable;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use serde_json::Number;
use tracing::warn;

use super::{SpendCacheUpdateError, added_spend};

pub(super) fn update_user_spend(
    user_id: &str,
    response_cost: Option<Decimal>,
    cached: &BTreeMap<String, Json>,
    queue: &mut Vec<(String, Json)>,
) {
    let Some(cost) = response_cost else {
        return;
    };
    if let Err(error) = user_updates(user_id, cost, cached, queue) {
        warn_group(
            "user",
            &format!("user_id={user_id}, response_cost={cost}"),
            &error,
        );
    }
}

// a missing or undecodable user row returns from this group, so the global proxy spend is not bumped either
fn user_updates(
    user_id: &str,
    cost: Decimal,
    cached: &BTreeMap<String, Json>,
    queue: &mut Vec<(String, Json)>,
) -> Result<(), SpendCacheUpdateError> {
    let Some(cached_user) = cached.get(user_id) else {
        return Ok(());
    };
    let Some(mut user) =
        payload::decode::<LiteLLMUserTable>(cached_user).map_err(SpendCacheUpdateError::Decode)?
    else {
        return Ok(());
    };
    user.spend = added_spend(user.spend, cost)?;
    queue.push((
        user_id.to_owned(),
        payload::encode(&user).map_err(SpendCacheUpdateError::Encode)?,
    ));
    let Some(global_spend) = cached.get(GLOBAL_PROXY_SPEND_CACHE_KEY) else {
        return Ok(());
    };
    let Json::Number(number) = global_spend else {
        return Ok(());
    };
    let current =
        Decimal::from_str(&number.to_string()).map_err(SpendCacheUpdateError::GlobalSpend)?;
    let total = added_spend(current, cost)?;
    let written = Number::from_str(&total.to_string())
        .map(Json::Number)
        .map_err(SpendCacheUpdateError::GlobalNumber)?;
    queue.push((GLOBAL_PROXY_SPEND_CACHE_KEY.to_owned(), written));
    Ok(())
}

pub(super) fn update_end_user_spend(
    end_user_id: &str,
    response_cost: Option<Decimal>,
    cached: &BTreeMap<String, Json>,
    queue: &mut Vec<(String, Json)>,
) {
    let Some(cost) = response_cost else {
        return;
    };
    if let Err(error) = end_user_updates(end_user_id, cost, cached, queue) {
        warn_group(
            "end user",
            &format!("end_user_id={end_user_id}, response_cost={cost}"),
            &error,
        );
    }
}

fn end_user_updates(
    end_user_id: &str,
    cost: Decimal,
    cached: &BTreeMap<String, Json>,
    queue: &mut Vec<(String, Json)>,
) -> Result<(), SpendCacheUpdateError> {
    let key = CacheKey::EndUserId(end_user_id).render();
    let Some(cached_end_user) = cached.get(&key) else {
        return Ok(());
    };
    let Some(mut end_user) = payload::decode::<LiteLLMEndUserTable>(cached_end_user)
        .map_err(SpendCacheUpdateError::Decode)?
    else {
        return Ok(());
    };
    end_user.spend = added_spend(end_user.spend, cost)?;
    let encoded = payload::encode(&end_user).map_err(SpendCacheUpdateError::Encode)?;
    queue.push((key, encoded));
    Ok(())
}

pub(super) fn update_team_spend(
    team_id: &str,
    response_cost: Option<Decimal>,
    cached: &BTreeMap<String, Json>,
    queue: &mut Vec<(String, Json)>,
) {
    let Some(cost) = response_cost else {
        return;
    };
    if let Err(error) = team_updates(team_id, cost, cached, queue) {
        warn_group(
            "team",
            &format!("team_id={team_id}, response_cost={cost}"),
            &error,
        );
    }
}

fn team_updates(
    team_id: &str,
    cost: Decimal,
    cached: &BTreeMap<String, Json>,
    queue: &mut Vec<(String, Json)>,
) -> Result<(), SpendCacheUpdateError> {
    let key = format!("team_id:{team_id}");
    let Some(cached_team) = cached.get(&key) else {
        return Ok(());
    };
    let Some(mut team) = payload::decode::<LiteLLMTeamTableCachedObj>(cached_team)
        .map_err(SpendCacheUpdateError::Decode)?
    else {
        return Ok(());
    };
    let existing = team.team.fields.spend.unwrap_or_default();
    team.team.fields.spend = Some(added_spend(existing, cost)?);
    let encoded = payload::encode(&team).map_err(SpendCacheUpdateError::Encode)?;
    queue.push((key, encoded));
    Ok(())
}

pub(super) fn update_spends_for_tags(
    tags: &[Json],
    response_cost: Option<Decimal>,
    cached: &BTreeMap<String, Json>,
    queue: &mut Vec<(String, Json)>,
) {
    let Some(cost) = response_cost else {
        return;
    };
    if let Err(error) = tag_updates(tags, cost, cached, queue) {
        warn_group(
            "tag",
            &format!("tags={tags:?}, response_cost={cost}"),
            &error,
        );
    }
}

fn tag_updates(
    tags: &[Json],
    cost: Decimal,
    cached: &BTreeMap<String, Json>,
    queue: &mut Vec<(String, Json)>,
) -> Result<(), SpendCacheUpdateError> {
    for tag in tags {
        let Json::Text(name) = tag else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let key = CacheKey::Tag(name.as_str()).render();
        let Some(cached_tag) = cached.get(&key) else {
            continue;
        };
        let Some(mut tag_row) =
            payload::decode::<TagTable>(cached_tag).map_err(SpendCacheUpdateError::Decode)?
        else {
            continue;
        };
        tag_row.spend = added_spend(tag_row.spend, cost)?;
        let encoded = payload::encode(&tag_row).map_err(SpendCacheUpdateError::Encode)?;
        queue.push((key, encoded));
    }
    Ok(())
}

fn warn_group(kind: &str, context: &str, error: &SpendCacheUpdateError) {
    warn!(
        "Spend tracking - failed to update {kind} spend in cache. Budget enforcement may use stale spend values. {context} - {error}"
    );
}
