pub mod client;
pub mod error;
pub mod job;
pub mod normalize;

use client::Connection;
use error::FineTuneError;
use job::{FineTuneCreate, FineTuneJob};

pub fn create_fine_tuning_job(
    connection: Connection<'_>,
    data: &FineTuneCreate,
) -> Result<FineTuneJob, FineTuneError> {
    let reply = connection
        .open()?
        .create(data)
        .map_err(|source| FineTuneError::Sdk { source })?;
    job::decode_fine_tune_job(&reply, false)
}

pub fn list_fine_tuning_jobs(
    connection: Connection<'_>,
    after: Option<&str>,
    limit: Option<i64>,
) -> Result<Vec<u8>, FineTuneError> {
    connection
        .open()?
        .list(after, limit)
        .map_err(|source| FineTuneError::Sdk { source })
}

pub fn retrieve_fine_tuning_job(
    connection: Connection<'_>,
    job_id: &str,
) -> Result<FineTuneJob, FineTuneError> {
    let reply = connection
        .open()?
        .retrieve(job_id)
        .map_err(|source| FineTuneError::Sdk { source })?;
    job::decode_fine_tune_job(&reply, false)
}

pub fn cancel_fine_tuning_job(
    connection: Connection<'_>,
    job_id: &str,
) -> Result<FineTuneJob, FineTuneError> {
    let reply = connection
        .open()?
        .cancel(job_id)
        .map_err(|source| FineTuneError::Sdk { source })?;
    job::decode_fine_tune_job(&reply, false)
}

mod tests;
