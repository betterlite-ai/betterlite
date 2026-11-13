mod tests;

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;

use crate::model_budget_values::batch_dedup::batch_id_to_charge_once;
use crate::model_budget_values::batch_keys::batch_charged_once_marker_key;
use crate::model_budget_values::budget_config::BudgetRuntime;
use crate::model_budget_values::budget_scope::BudgetScope;
use crate::model_budget_values::resolution::{ResolvedModelBudget, resolve_model_budget};
use crate::model_budget_values::spend_keys::model_budget_spend_cache_key;
// ast-grep-ignore: no-long-names
use crate::model_budget_values::start_time_keys::model_budget_start_time_cache_key as start_time_cache_key;
use crate::model_budget_values::team_override::team_model_budget_applies;

use super::ModelBudgetLimiter;
use super::cache::ModelBudgetCache;
use super::error::ChargeError;

// kwargs["litellm_params"]["metadata"], the budgets the key carried into the call
#[derive(Clone, Copy)]
pub struct ChargeMetadata<'input> {
    pub key_max_budget: Option<&'input BTreeMap<String, JsonData>>,
    pub team_max_budget: Option<&'input BTreeMap<String, JsonData>>,
    pub user_max_budget: Option<&'input BTreeMap<String, JsonData>>,
    pub end_user_max_budget: Option<&'input BTreeMap<String, JsonData>>,
}

// standard_logging_payload["metadata"] plus its top-level end_user
#[derive(Clone, Copy)]
pub struct PayloadIdentity<'input> {
    pub key_hash: Option<&'input str>,
    pub team_id: Option<&'input str>,
    pub user_id: Option<&'input str>,
    pub end_user: Option<&'input str>,
    pub end_user_id: Option<&'input str>,
}

// the fields async_log_success_event reads off the standard logging payload and its kwargs
#[derive(Clone, Copy)]
pub struct SuccessEvent<'input> {
    pub model_group: Option<&'input str>,
    pub model: Option<&'input str>,
    pub response_cost: Decimal,
    pub metadata: ChargeMetadata<'input>,
    pub payload: PayloadIdentity<'input>,
    pub call_type: Option<&'input str>,
    pub batch_id: Option<&'input str>,
}

#[derive(Clone, Copy)]
struct EntityBudget<'input> {
    scope: BudgetScope,
    entity_id: Option<&'input str>,
    model_max_budget: Option<&'input BTreeMap<String, JsonData>>,
}

struct ChargeableEntity<'input> {
    scope: BudgetScope,
    entity_id: &'input str,
    budget: ResolvedModelBudget,
}

pub async fn log_success_event<C>(
    limiter: &ModelBudgetLimiter<C>,
    event: Option<&SuccessEvent<'_>>,
) -> Result<(), ChargeError<C::Error>>
where
    C: ModelBudgetCache,
{
    let Some(payload) = event else {
        return Ok(());
    };
    let Some(model) = payload.model_group.or(payload.model) else {
        return Ok(());
    };
    let runtime = BudgetRuntime {
        durations: limiter.durations.as_ref(),
        now: limiter.clock.now(),
    };
    let team_applies = team_model_budget_applies(
        model,
        payload.metadata.key_max_budget,
        limiter.catalog.as_ref(),
        runtime,
    );
    let entities = entity_budgets(payload, team_applies);
    let chargeable = resolve_entity_model_budgets(limiter, model, &entities, runtime);
    if chargeable.is_empty() {
        return Ok(());
    }
    let claimed_batch = batch_id_to_charge_once(
        payload.call_type.unwrap_or_default(),
        payload.batch_id,
        payload.response_cost,
    );
    for entity in &chargeable {
        charge_entity(limiter, entity, payload.response_cost, claimed_batch).await?;
    }
    flush_increments(limiter).await;
    Ok(())
}

fn entity_budgets<'input>(
    payload: &SuccessEvent<'input>,
    team_applies: bool,
) -> [EntityBudget<'input>; 4] {
    [
        EntityBudget {
            scope: BudgetScope::Key,
            entity_id: payload.payload.key_hash,
            model_max_budget: payload.metadata.key_max_budget,
        },
        EntityBudget {
            scope: BudgetScope::Team,
            entity_id: payload.payload.team_id,
            model_max_budget: if team_applies {
                payload.metadata.team_max_budget
            } else {
                None
            },
        },
        EntityBudget {
            scope: BudgetScope::User,
            entity_id: payload.payload.user_id,
            model_max_budget: payload.metadata.user_max_budget,
        },
        EntityBudget {
            scope: BudgetScope::EndUser,
            entity_id: payload.payload.end_user.or(payload.payload.end_user_id),
            model_max_budget: payload.metadata.end_user_max_budget,
        },
    ]
}

fn resolve_entity_model_budgets<'input, C>(
    limiter: &ModelBudgetLimiter<C>,
    model: &str,
    entities: &[EntityBudget<'input>],
    runtime: BudgetRuntime<'_>,
) -> Vec<ChargeableEntity<'input>>
where
    C: ModelBudgetCache,
{
    let mut chargeable = Vec::new();
    for entity in entities.iter().copied() {
        let Some(entity_id) = entity.entity_id else {
            continue;
        };
        let Some(budget) = entity.model_max_budget.filter(|budget| !budget.is_empty()) else {
            continue;
        };
        let Some(resolved) = resolve_model_budget(model, budget, limiter.catalog.as_ref(), runtime)
        else {
            continue;
        };
        chargeable.push(ChargeableEntity {
            scope: entity.scope,
            entity_id,
            budget: resolved,
        });
    }
    chargeable
}

async fn flush_increments<C>(limiter: &ModelBudgetLimiter<C>)
where
    C: ModelBudgetCache,
{
    if limiter.cache.redis_configured() {
        limiter.cache.flush_increments().await;
    }
}

async fn charge_entity<C>(
    limiter: &ModelBudgetLimiter<C>,
    entity: &ChargeableEntity<'_>,
    response_cost: Decimal,
    batch_id: Option<&str>,
) -> Result<(), ChargeError<C::Error>>
where
    C: ModelBudgetCache,
{
    let spec = &entity.budget.budget_config.budget_duration;
    let seconds = limiter
        .durations
        .seconds(spec, limiter.clock.now())
        .ok_or_else(|| ChargeError::Duration { spec: spec.clone() })?;
    let Ok(window_secs) = u64::try_from(seconds) else {
        return Err(ChargeError::Duration { spec: spec.clone() });
    };
    let duration = Duration::from_secs(window_secs);
    let spend_key = model_budget_spend_cache_key(
        entity.scope,
        entity.entity_id,
        &entity.budget.budget_model,
        spec,
    );
    if let Some(claimed) = batch_id {
        let marker_key = batch_charged_once_marker_key(&spend_key, claimed);
        if limiter.cache.increment_counter(&marker_key, duration).await != Some(Decimal::ONE) {
            return Ok(());
        }
    }
    let window_start_key = start_time_cache_key(
        entity.scope,
        entity.entity_id,
        &entity.budget.budget_model,
        spec,
    );
    limiter
        .cache
        .increment_spend(duration, &spend_key, &window_start_key, response_cost)
        .await
        .map_err(|source| ChargeError::Increment {
            spend_key: spend_key.clone(),
            source,
        })
}
