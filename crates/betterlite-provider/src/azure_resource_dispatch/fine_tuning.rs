use std::future::{Future, ready};
use std::pin::Pin;

use betterlite_azure::fine_tuning::client::{ClientMaterializer, Connection};
use betterlite_azure::fine_tuning::error::AzureFineTuneError;
use betterlite_azure::fine_tuning::{
    create_fine_tuning_job, list_fine_tuning_jobs, request_fine_tuning_job,
};
use betterlite_openai::fine_tuning::JobRequest;
use betterlite_openai::fine_tuning::job::{FineTuneCreate, FineTuneJob};

// The create call takes the caller's data because it defaults trainingType in place
pub enum Call<'call> {
    Create(&'call mut FineTuneCreate),
    Retrieve(&'call str),
    Cancel(&'call str),
    List {
        after: Option<&'call str>,
        limit: Option<i64>,
    },
}

pub enum Outcome {
    Job(Box<FineTuneJob>),
    Page(Vec<u8>),
}

pub struct Dispatch<'cache, 'data, 'call, M: ClientMaterializer> {
    pub connection: Connection<'cache, 'data, M>,
    pub call: Call<'call>,
}

pub type RunFuture<'call> =
    Pin<Box<dyn Future<Output = Result<Outcome, AzureFineTuneError>> + Send + 'call>>;

impl<M: ClientMaterializer> Dispatch<'_, '_, '_, M> {
    // B542's four job calls; the list keeps the SDK page bytes because only job replies are normalized
    pub fn run(self) -> Result<Outcome, AzureFineTuneError> {
        let Self { connection, call } = self;
        Ok(match call {
            Call::Create(data) => Outcome::Job(Box::new(create_fine_tuning_job(connection, data)?)),
            Call::Retrieve(job_id) => Outcome::Job(Box::new(request_fine_tuning_job(
                connection,
                &JobRequest::Retrieve(job_id),
            )?)),
            Call::Cancel(job_id) => Outcome::Job(Box::new(request_fine_tuning_job(
                connection,
                &JobRequest::Cancel(job_id),
            )?)),
            Call::List { after, limit } => {
                Outcome::Page(list_fine_tuning_jobs(connection, after, limit)?)
            }
        })
    }
}

pub fn dispatch<'cache, 'data, 'call, M>(
    dispatch: Dispatch<'cache, 'data, 'call, M>,
) -> RunFuture<'call>
where
    M: ClientMaterializer,
{
    Box::pin(ready(dispatch.run()))
}
