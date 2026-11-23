use std::slice;

use chrono::{DateTime, Utc};

use crate::shadow_eval_lifecycle::error::LifecycleError;
use crate::shadow_eval_lifecycle::response::{apply_target_labels, job_from_legs};
use crate::shadow_eval_lifecycle::store::ShadowLifecycleStore;
use crate::shadow_eval_models::job::ShadowJobResponse;
use crate::shadow_eval_models::{ShadowEvalStatus, ShadowEvalTargetType};
use crate::shadow_result_aggregation::labels::target_ids_of;

// stop_shadow_eval_job: the statement claims the job only while a leg still samples, then the response reports what happened
pub async fn stop_shadow_eval_job<Store>(
    store: &Store,
    job_id: &str,
    caller: Option<&str>,
    now: DateTime<Utc>,
) -> Result<ShadowJobResponse, LifecycleError>
where
    Store: ShadowLifecycleStore + ?Sized,
{
    let operator = caller.unwrap_or("operator");
    let claimed = store
        .stop_job(job_id, operator, now)
        .await
        .map_err(|error| LifecycleError::Database(Box::new(error)))?;
    let legs = store
        .legs(job_id)
        .await
        .map_err(|error| LifecycleError::Database(Box::new(error)))?;
    if legs.is_empty() {
        return Err(LifecycleError::JobNotFound {
            job_id: job_id.to_owned(),
        });
    }
    let leg_ids: Vec<String> = legs.iter().map(|leg| leg.id.clone()).collect();
    let stats = store
        .attempt_stats(&leg_ids)
        .await
        .map_err(|error| LifecycleError::Database(Box::new(error)))?;
    let mut job = job_from_legs(job_id, &legs, Some(&stats))?;
    let current = job.response(now)?;
    if claimed == 0 {
        return Err(LifecycleError::NotStoppable {
            job_id: job_id.to_owned(),
            status: status_text(current.status).to_owned(),
        });
    }
    let keys = target_ids_of(slice::from_ref(&job), ShadowEvalTargetType::Key);
    let teams = target_ids_of(slice::from_ref(&job), ShadowEvalTargetType::Team);
    let users = target_ids_of(slice::from_ref(&job), ShadowEvalTargetType::User);
    let rows = store
        .load_targets(&keys, &teams, &users)
        .await
        .map_err(|error| LifecycleError::Database(Box::new(error)))?;
    apply_target_labels(&mut job, &rows);
    Ok(job.response(now)?)
}

// the Literal text the source's f-string prints for a job's derived status
const fn status_text(status: ShadowEvalStatus) -> &'static str {
    match status {
        ShadowEvalStatus::Running => "running",
        ShadowEvalStatus::Completed => "completed",
        ShadowEvalStatus::Stopped => "stopped",
    }
}
