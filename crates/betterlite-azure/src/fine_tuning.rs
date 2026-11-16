pub mod client;
pub mod error;

use betterlite_openai::fine_tuning::job::{FineTuneCreate, FineTuneJob, decode_fine_tune_job};

use client::{ClientMaterializer, Connection};
use error::AzureFineTuneError;

pub enum JobRequest<'request> {
    Create(&'request FineTuneCreate),
    Retrieve(&'request str),
    Cancel(&'request str),
}

// Azure needs a trainingType in extra_body and defaults to 1, supervised; an explicit value, even 0 or false, stays
pub fn ensure_training_type(data: &mut FineTuneCreate) {
    let explicit = data
        .extra
        .get("extra_body")
        .and_then(|body| body.as_object())
        .and_then(|body| body.get("trainingType"))
        .is_some_and(|value| !value.is_null());
    if explicit {
        return;
    }
    let mut body = data
        .extra
        .get("extra_body")
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default();
    body.insert("trainingType".to_owned(), serde_json::json!(1));
    data.extra
        .insert("extra_body".to_owned(), serde_json::json!(body));
    tracing::debug!("Azure fine-tuning: defaulting trainingType=1 (supervised)");
}

pub fn request_fine_tuning_job<M: ClientMaterializer>(
    connection: Connection<'_, '_, M>,
    request: &JobRequest<'_>,
) -> Result<FineTuneJob, AzureFineTuneError> {
    let is_async = connection.is_async;
    let client = connection.open()?;
    let reply = match request {
        JobRequest::Create(data) => {
            if !is_async {
                tracing::debug!("creating fine tuning job, args= {data:?}");
            }
            client.create(data)
        }
        JobRequest::Retrieve(job_id) => client.retrieve(job_id),
        JobRequest::Cancel(job_id) => client.cancel(job_id),
    };
    let bytes = reply.map_err(|source| AzureFineTuneError::Sdk { source })?;
    decode_fine_tune_job(&bytes, true).map_err(AzureFineTuneError::from)
}

pub fn create_fine_tuning_job<M: ClientMaterializer>(
    connection: Connection<'_, '_, M>,
    data: &mut FineTuneCreate,
) -> Result<FineTuneJob, AzureFineTuneError> {
    ensure_training_type(data);
    request_fine_tuning_job(connection, &JobRequest::Create(data))
}

pub fn retrieve_fine_tuning_job<M: ClientMaterializer>(
    connection: Connection<'_, '_, M>,
    job_id: &str,
) -> Result<FineTuneJob, AzureFineTuneError> {
    request_fine_tuning_job(connection, &JobRequest::Retrieve(job_id))
}

pub fn cancel_fine_tuning_job<M: ClientMaterializer>(
    connection: Connection<'_, '_, M>,
    job_id: &str,
) -> Result<FineTuneJob, AzureFineTuneError> {
    request_fine_tuning_job(connection, &JobRequest::Cancel(job_id))
}

// the inherited list returns the SDK page as it is; only single job replies carry the Azure normalization
pub fn list_fine_tuning_jobs<M: ClientMaterializer>(
    connection: Connection<'_, '_, M>,
    after: Option<&str>,
    limit: Option<i64>,
) -> Result<Vec<u8>, AzureFineTuneError> {
    let is_async = connection.is_async;
    let client = connection.open()?;
    if !is_async {
        tracing::debug!("list fine tuning job, after= {after:?}, limit= {limit:?}");
    }
    client
        .list(after, limit)
        .map_err(|source| AzureFineTuneError::Sdk { source })
}

mod tests;
