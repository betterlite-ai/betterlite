use betterlite_lens::models::job::{Job, JobStatus, Lens, Worker};
use chrono::{DateTime, Utc};

use crate::lens_access::lookup::{get_lens, lens_repository};
use crate::lens_access::state::{LensDatabase, LensRoutesState};
use crate::lens_inference::jobs::current_job;
use crate::lens_worker_routes::error::LensWorkerError;

// assigned: the lens is visible to the worker's scope and its current job is running under a live lease
pub(crate) async fn assigned<Db, Store>(
    state: &LensRoutesState<Db, Store>,
    lens_id: &str,
    job_id: &str,
    worker: &Worker,
    now: DateTime<Utc>,
) -> Result<(Lens, Job), LensWorkerError>
where
    Db: LensDatabase,
{
    let lens = get_lens(lens_repository(state)?, lens_id, &worker.scope).await?;
    let owned = current_job(&lens).filter(|job| {
        job.id == job_id
            && job.status == JobStatus::Running
            && job.worker_id.as_deref() == Some(worker.id.as_str())
            && job.lease_until.is_some_and(|lease| lease > now)
    });
    let Some(job) = owned.cloned() else {
        return Err(LensWorkerError::NotOwned);
    };
    Ok((lens, job))
}

mod tests;
