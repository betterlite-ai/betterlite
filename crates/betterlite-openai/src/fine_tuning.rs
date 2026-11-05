pub mod client;
pub mod error;
pub mod job;
pub mod normalize;

use client::Connection;
use error::FineTuneError;
use job::{FineTuneCreate, FineTuneJob};

pub enum JobRequest<'request> {
    Create(&'request FineTuneCreate),
    Retrieve(&'request str),
    Cancel(&'request str),
}

pub fn request_fine_tuning_job(
    connection: Connection<'_>,
    request: &JobRequest<'_>,
) -> Result<FineTuneJob, FineTuneError> {
    let client = connection.open()?;
    let reply = match request {
        JobRequest::Create(data) => client.create(data),
        JobRequest::Retrieve(job_id) => client.retrieve(job_id),
        JobRequest::Cancel(job_id) => client.cancel(job_id),
    };
    job::decode_fine_tune_job(
        &reply.map_err(|source| FineTuneError::Sdk { source })?,
        false,
    )
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

mod tests;
