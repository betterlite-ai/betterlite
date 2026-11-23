use betterlite_models::judge_primitives::provider_qualified::ProviderCache;
use betterlite_models::resolution::ResolutionContext;
use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

use crate::judge_targets::sdk::JudgeSdk;
use crate::shadow_eval_job_loading::jobs::ShadowEvalDirection as JobDirection;
use crate::shadow_eval_lifecycle::error::LifecycleError;
use crate::shadow_eval_lifecycle::response::{apply_target_labels, job_from_legs};
use crate::shadow_eval_lifecycle::store::{
    ClaimedTarget, InsertRejection, NewLeg, ShadowLifecycleStore, TargetRows,
};
use crate::shadow_eval_models::job::ShadowJobResponse;
use crate::shadow_eval_models::{ShadowEvalModelError, ShadowEvalTargetType};
use crate::shadow_eval_validation::request::{
    SHADOW_EVAL_TURN_VALVE, ShadowEvalDirection as RequestDirection, StartShadowEvalRequest,
};
use crate::shadow_eval_validation::router_view::{AnthropicCredentials, ShadowEvalRouter};
use crate::shadow_eval_validation::start::{
    StartContext, TargetKey, TargetRecords, validate_start,
};
use crate::shadow_result_aggregation::targets::target_type_name;

// everything B345's validate_start reads, passed down from the server state
pub struct StartDeps<'deps> {
    pub router: Option<ShadowEvalRouter<'deps>>,
    pub sdk: &'deps dyn JudgeSdk,
    pub cache: &'deps ProviderCache,
    pub resolution: &'deps ResolutionContext<'deps>,
    pub credentials: AnthropicCredentials<'deps>,
}

// the caller, clock and id source one start request runs with
pub struct StartCall<'call, Ids> {
    pub caller: Option<&'call str>,
    pub now: DateTime<Utc>,
    pub ids: &'call mut Ids,
}

const fn job_direction(direction: RequestDirection) -> JobDirection {
    match direction {
        RequestDirection::Forward => JobDirection::Forward,
        RequestDirection::Reverse => JobDirection::Reverse,
    }
}

// start_shadow_eval: validate, sweep finished slots, refuse a claimed target, then write every leg in one insert
pub async fn start_shadow_eval<Store, Ids>(
    store: &Store,
    deps: StartDeps<'_>,
    request: &StartShadowEvalRequest,
    call: &mut StartCall<'_, Ids>,
) -> Result<ShadowJobResponse, LifecycleError>
where
    Store: ShadowLifecycleStore + ?Sized,
    Ids: Iterator<Item = Uuid>,
{
    let target_rows = store
        .load_targets(&request.api_key_ids, &request.team_ids, &request.user_ids)
        .await
        .map_err(|error| LifecycleError::Database(Box::new(error)))?;
    validate_against(request, deps, &target_rows)?;
    let direction = job_direction(request.direction);
    let requested = requested_by_type(request);
    sweep_requested(store, &requested).await?;
    reject_claimed(store, &requested, direction).await?;
    let (group_id, legs) = mint_legs(call, request, direction, &requested)?;
    if let Err(rejection) = store.create_legs(&legs).await {
        return Err(match rejection {
            InsertRejection::UniqueViolation => LifecycleError::ConcurrentClaim {
                direction: direction.as_str().to_owned(),
            },
            InsertRejection::Database(error) => LifecycleError::Database(Box::new(error)),
        });
    }
    let leg_ids: Vec<String> = legs.iter().map(|leg| leg.id.clone()).collect();
    if let Err(error) = store.seed_funnel(&leg_ids).await {
        tracing::error!("shadow_eval: funnel seed failed for job {group_id}: {error}");
    }
    let rows: Vec<_> = legs.iter().map(NewLeg::row).collect();
    let mut job = job_from_legs(&group_id, &rows, None)?;
    apply_target_labels(&mut job, &target_rows);
    Ok(job.response(call.now)?)
}

fn validate_against(
    request: &StartShadowEvalRequest,
    deps: StartDeps<'_>,
    target_rows: &TargetRows,
) -> Result<(), LifecycleError> {
    let keys: Vec<TargetKey<'_>> = target_rows
        .keys
        .iter()
        .map(|row| TargetKey {
            token: row.token.as_str(),
            team_id: row.team_id.as_deref(),
        })
        .collect();
    let teams: Vec<&str> = target_rows
        .teams
        .iter()
        .map(|row| row.team_id.as_str())
        .collect();
    let users: Vec<&str> = target_rows
        .users
        .iter()
        .map(|row| row.user_id.as_str())
        .collect();
    let context = StartContext {
        router: deps.router,
        sdk: deps.sdk,
        cache: deps.cache,
        resolution: deps.resolution,
        credentials: deps.credentials,
        records: TargetRecords {
            keys: &keys,
            teams: &teams,
            users: &users,
        },
    };
    validate_start(request, &context).map_err(LifecycleError::from)
}

// the sweep runs once per requested target kind, so a finished job never holds a slot against a new one
async fn sweep_requested<Store>(
    store: &Store,
    requested: &[(ShadowEvalTargetType, Vec<String>)],
) -> Result<(), LifecycleError>
where
    Store: ShadowLifecycleStore + ?Sized,
{
    for (target_type, ids) in requested {
        store
            .sweep_finished(ids, *target_type)
            .await
            .map_err(|error| LifecycleError::Database(Box::new(error)))?;
    }
    Ok(())
}

async fn reject_claimed<Store>(
    store: &Store,
    requested: &[(ShadowEvalTargetType, Vec<String>)],
    direction: JobDirection,
) -> Result<(), LifecycleError>
where
    Store: ShadowLifecycleStore + ?Sized,
{
    let claimed: Vec<ClaimedTarget> = store
        .claimed_targets(requested, direction)
        .await
        .map_err(|error| LifecycleError::Database(Box::new(error)))?;
    if claimed.is_empty() {
        return Ok(());
    }
    let mut described: Vec<String> = claimed
        .iter()
        .map(|row| {
            format!(
                "{} {} (job {})",
                target_type_name(row.target_type),
                row.target_id,
                row.group_id
            )
        })
        .collect();
    described.sort();
    Err(LifecycleError::AlreadyClaimed {
        direction: direction.as_str().to_owned(),
        targets: described.join(", "),
    })
}

fn requested_by_type(request: &StartShadowEvalRequest) -> Vec<(ShadowEvalTargetType, Vec<String>)> {
    [
        (ShadowEvalTargetType::Key, &request.api_key_ids),
        (ShadowEvalTargetType::Team, &request.team_ids),
        (ShadowEvalTargetType::User, &request.user_ids),
    ]
    .into_iter()
    .filter(|(_, ids)| !ids.is_empty())
    .map(|(target_type, ids)| (target_type, ids.clone()))
    .collect()
}

// a job's id is minted once, with its leg ids, before the single insert; an ambiguous write is never retried
fn mint_legs<Ids>(
    call: &mut StartCall<'_, Ids>,
    request: &StartShadowEvalRequest,
    direction: JobDirection,
    requested: &[(ShadowEvalTargetType, Vec<String>)],
) -> Result<(String, Vec<NewLeg>), LifecycleError>
where
    Ids: Iterator<Item = Uuid>,
{
    let router_name = request
        .router_names
        .first()
        .cloned()
        .ok_or(LifecycleError::Model(ShadowEvalModelError::NoRouterArms))?;
    let ids = &mut *call.ids;
    let group_id = ids.next().ok_or(LifecycleError::IdsExhausted)?.to_string();
    let window = Duration::try_days(i64::from(request.duration_days))
        .ok_or(LifecycleError::WindowOverflow)?;
    let ends_at = call
        .now
        .checked_add_signed(window)
        .ok_or(LifecycleError::WindowOverflow)?;
    let max_turns = i32::try_from(SHADOW_EVAL_TURN_VALVE).unwrap_or(i32::MAX);
    let mut legs = Vec::new();
    for (target_type, target_ids) in requested {
        for target_id in target_ids {
            let id = ids.next().ok_or(LifecycleError::IdsExhausted)?.to_string();
            legs.push(NewLeg {
                id,
                group_id: group_id.clone(),
                target_type: *target_type,
                target_id: target_id.clone(),
                router_name: router_name.clone(),
                router_names: request.router_names.clone(),
                models: request.models.clone(),
                direction,
                baseline_model: request.baseline_model.clone(),
                judge_model: request.judge_model.clone(),
                shadow_percentage: request.shadow_percentage,
                max_turns,
                max_budget: request.max_budget,
                created_by: call.caller.map(str::to_owned),
                created_at: call.now,
                ends_at,
            });
        }
    }
    Ok((group_id, legs))
}
