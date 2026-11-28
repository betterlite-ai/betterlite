use betterlite_lens::models::job::{Job, JobStatus, Lens};

// state.current_job: the job a worker is running or about to run, which reserve checks against
#[must_use]
pub fn current_job(lens: &Lens) -> Option<&Job> {
    lens.jobs
        .iter()
        .find(|job| matches!(job.status, JobStatus::Queued | JobStatus::Running))
}

// state.replace_job: the job with the same id takes the new value, and every other job stays
#[must_use]
pub fn replace_job(lens: &Lens, job: &Job) -> Lens {
    let mut replaced = lens.clone();
    replaced.jobs = lens
        .jobs
        .iter()
        .map(|current| {
            if current.id == job.id {
                job.clone()
            } else {
                current.clone()
            }
        })
        .collect();
    replaced
}
