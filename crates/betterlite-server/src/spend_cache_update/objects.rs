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
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Number;
use tracing::warn;

use super::{SpendCacheUpdateError, UpdateInput, added_spend};

// the row types whose cached spend a request's cost raises
trait SpendRow {
    fn spend_mut(&mut self) -> &mut Decimal;
}

impl SpendRow for LiteLLMUserTable {
    fn spend_mut(&mut self) -> &mut Decimal {
        &mut self.spend
    }
}

impl SpendRow for LiteLLMEndUserTable {
    fn spend_mut(&mut self) -> &mut Decimal {
        &mut self.spend
    }
}

impl SpendRow for LiteLLMTeamTableCachedObj {
    fn spend_mut(&mut self) -> &mut Decimal {
        self.team.fields.spend.get_or_insert(Decimal::ZERO)
    }
}

impl SpendRow for TagTable {
    fn spend_mut(&mut self) -> &mut Decimal {
        &mut self.spend
    }
}

pub(super) fn update_objects(
    input: &UpdateInput,
    cached: &BTreeMap<String, Json>,
    queue: &mut Vec<(String, Json)>,
) {
    let Some(cost) = input.response_cost else {
        return;
    };
    if let Some(user_id) = input.user_id.as_deref() {
        let context = format!("user_id={user_id}, response_cost={cost}");
        // a missing or undecodable user row returns from this group, so the global proxy spend is not bumped either
        let outcome = match add_cost_to_cached::<LiteLLMUserTable>(user_id, cost, cached, queue) {
            Ok(true) => bump_global_spend(cost, cached, queue),
            Ok(false) => Ok(()),
            Err(error) => Err(error),
        };
        report_group("user", &context, outcome);
    }
    if let Some(end_user_id) = input.end_user_id.as_deref() {
        let key = CacheKey::EndUserId(end_user_id).render();
        let context = format!("end_user_id={end_user_id}, response_cost={cost}");
        let outcome = add_cost_to_cached::<LiteLLMEndUserTable>(&key, cost, cached, queue);
        report_group("end user", &context, outcome);
    }
    if let Some(team_id) = input.team_id.as_deref() {
        let key = format!("team_id:{team_id}");
        let context = format!("team_id={team_id}, response_cost={cost}");
        let outcome = add_cost_to_cached::<LiteLLMTeamTableCachedObj>(&key, cost, cached, queue);
        report_group("team", &context, outcome);
    }
    if let Some(tags) = input.tags.as_deref() {
        let context = format!("tags={tags:?}, response_cost={cost}");
        let outcome = tag_updates(tags, cost, cached, queue);
        report_group("tag", &context, outcome);
    }
}

fn bump_global_spend(
    cost: Decimal,
    cached: &BTreeMap<String, Json>,
    queue: &mut Vec<(String, Json)>,
) -> Result<(), SpendCacheUpdateError> {
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

// false when the row is not cached or does not decode, so the user branch can skip the global spend too
fn add_cost_to_cached<Row>(
    key: &str,
    cost: Decimal,
    cached: &BTreeMap<String, Json>,
    queue: &mut Vec<(String, Json)>,
) -> Result<bool, SpendCacheUpdateError>
where
    Row: SpendRow + Serialize + DeserializeOwned,
{
    let Some(cached_row) = cached.get(key) else {
        return Ok(false);
    };
    let Some(mut row) =
        payload::decode::<Row>(cached_row).map_err(SpendCacheUpdateError::Decode)?
    else {
        return Ok(false);
    };
    let spend = row.spend_mut();
    *spend = added_spend(*spend, cost)?;
    queue.push((
        key.to_owned(),
        payload::encode(&row).map_err(SpendCacheUpdateError::Encode)?,
    ));
    Ok(true)
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
        let _queued = add_cost_to_cached::<TagTable>(&key, cost, cached, queue)?;
    }
    Ok(())
}

fn report_group<Outcome>(
    kind: &str,
    context: &str,
    outcome: Result<Outcome, SpendCacheUpdateError>,
) {
    if let Err(error) = outcome {
        warn!(
            "Spend tracking - failed to update {kind} spend in cache. Budget enforcement may use stale spend values. {context} - {error}"
        );
    }
}
