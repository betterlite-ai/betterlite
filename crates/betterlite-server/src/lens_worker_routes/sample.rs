use std::error::Error as StdError;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, header};
use betterlite_db::lens_repository::update::{DEFAULT_ATTEMPTS, LensUpdateError};
use betterlite_lens::models::execution::{Execution, Sample};
use betterlite_lens::models::job::{Job, Lens, Worker};
use betterlite_lens::sources::source_reader::SourceReader;
use betterlite_lens::sources::source_reader_sample::SampleWindow;
use betterlite_lens::sources::storage::Storage;

use crate::lens_access::lookup::{lens_repository, required};
use crate::lens_access::state::{LensDatabase, LensRoutesState, LensStorage};
use crate::lens_access::storage::source_reader;
use crate::lens_inference::jobs::{current_job, replace_job};
use crate::lens_worker_routes::WorkerRoutesState;
use crate::lens_worker_routes::credential::worker_auth;
use crate::lens_worker_routes::error::{JobMoved, LensWorkerError};
use crate::lens_worker_routes::lease::assigned;

// LiteLLM's SourceReader.sample page size; the route never sets it
const SAMPLE_PAGE_SIZE: usize = 100;

pub(crate) async fn sample<Db, Store>(
    State(state): State<WorkerRoutesState<Db, Store>>,
    Path((lens_id, job_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Sample>, LensWorkerError<<Store as Storage>::Error>>
where
    Db: LensDatabase,
    Store: LensStorage,
{
    let worker = worker_auth(&state.routes, headers.get(header::AUTHORIZATION)).await?;
    let now = state.routes.clock.now();
    let (lens, job) = assigned(&state.routes, &lens_id, &job_id, &worker, now).await?;
    if let Some(stored) = &job.sample {
        return Ok(Json(stored.clone()));
    }
    let reader = source_reader(state.routes.storage.as_ref())?;
    let built = gather(&reader, &lens, &job).await?;
    freeze(&state.routes, &lens_id, &job_id, &worker, &built).await
}

// the sample route's empty sample, for a job whose selection has not been frozen yet
pub(crate) const fn empty_sample() -> Sample {
    Sample {
        executions: Vec::new(),
        eligible: 0,
        selected: 0,
        next_offset: None,
        next_cursor: None,
    }
}

// the first page's window, in whole milliseconds from the job's integer microsecond bounds
const fn base_window(job: &Job) -> SampleWindow {
    SampleWindow {
        start: job.start.timestamp_micros().div_euclid(1000),
        end: job.end.timestamp_micros().div_euclid(1000),
        offset: 0,
        page_size: SAMPLE_PAGE_SIZE,
        preview: false,
        cursor: String::new(),
    }
}

const fn next_window(base: &SampleWindow, cursor: String) -> SampleWindow {
    SampleWindow {
        start: base.start,
        end: base.end,
        offset: base.offset,
        page_size: base.page_size,
        preview: base.preview,
        cursor,
    }
}

fn frozen_from(pages: Vec<Sample>) -> Sample {
    let eligible = pages.first().map_or(0, |page| page.eligible);
    let executions: Vec<Execution> = pages.into_iter().flat_map(|page| page.executions).collect();
    Sample {
        selected: i64::try_from(executions.len()).unwrap_or(i64::MAX),
        executions,
        eligible,
        next_offset: None,
        next_cursor: None,
    }
}

// page by cursor until the first page's selected count is reached or the source runs out
async fn gather<Store>(
    reader: &SourceReader<Store>,
    lens: &Lens,
    job: &Job,
) -> Result<Sample, LensWorkerError<<Store as Storage>::Error>>
where
    Store: Storage + Sync,
{
    let base = base_window(job);
    let mut pages: Vec<Sample> = Vec::new();
    let mut target: Option<i64> = None;
    loop {
        let cursor = pages
            .last()
            .and_then(|page| page.next_cursor.clone())
            .unwrap_or_default();
        let page = reader
            .sample(&lens.scope, &job.settings, next_window(&base, cursor))
            .await?;
        let wanted = target.unwrap_or(page.selected);
        target = Some(wanted);
        let gathered = pages
            .iter()
            .map(|stored| stored.executions.len())
            .sum::<usize>()
            .saturating_add(page.executions.len());
        // no cursor follows, or the executions gathered so far reach the first page's selected
        let done =
            page.next_cursor.is_none() || i64::try_from(gathered).unwrap_or(i64::MAX) >= wanted;
        pages.push(page);
        if done {
            break;
        }
    }
    Ok(frozen_from(pages))
}

async fn freeze<Db, Store>(
    state: &LensRoutesState<Db, Store>,
    lens_id: &str,
    job_id: &str,
    worker: &Worker,
    built: &Sample,
) -> Result<Json<Sample>, LensWorkerError<<Store as Storage>::Error>>
where
    Db: LensDatabase,
    Store: LensStorage,
{
    let freeze = |lens: Lens| -> Result<Lens, JobMoved> { frozen(lens, job_id, &worker.id, built) };
    let updated = lens_repository(state)?
        .update(lens_id, freeze, DEFAULT_ATTEMPTS, false)
        .await
        .map_err(moved_update)?;
    let lens = required(updated)?;
    let stored = lens
        .jobs
        .iter()
        .find(|job| job.id == job_id)
        .and_then(|job| job.sample.clone());
    let frozen_sample = stored.ok_or(LensWorkerError::SampleNotFrozen)?;
    Ok(Json(frozen_sample))
}

// another request that froze the sample first keeps its own selection
fn frozen(lens: Lens, job_id: &str, worker_id: &str, built: &Sample) -> Result<Lens, JobMoved> {
    let Some(active) = current_job(&lens) else {
        return Err(JobMoved);
    };
    if active.id != job_id || active.worker_id.as_deref() != Some(worker_id) {
        return Err(JobMoved);
    }
    if active.sample.is_some() {
        return Ok(lens);
    }
    let updated = Job {
        sample: Some(built.clone()),
        ..active.clone()
    };
    Ok(replace_job(&lens, &updated))
}

fn moved_update<E>(error: LensUpdateError<JobMoved>) -> LensWorkerError<E>
where
    E: StdError + 'static,
{
    match error {
        LensUpdateError::Repository(source) => LensWorkerError::Repository(source),
        LensUpdateError::Transform(JobMoved) => LensWorkerError::JobMoved,
    }
}

mod tests;
