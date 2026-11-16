use betterlite_openai::fine_tuning::client::Connection;
use betterlite_openai::fine_tuning::error::FineTuneError;
use betterlite_openai::fine_tuning::job::{FineTuneCreate, FineTuneJob};
use betterlite_openai::fine_tuning::{JobRequest, list_fine_tuning_jobs, request_fine_tuning_job};

pub struct Dispatch<'call> {
    pub connection: Connection<'call>,
    pub call: Call<'call>,
}

// B518's four lifecycle calls; list hands back the SDK page bytes it decoded nothing in
pub enum Call<'call> {
    Create(&'call FineTuneCreate),
    Retrieve(&'call str),
    Cancel(&'call str),
    List {
        after: Option<&'call str>,
        limit: Option<i64>,
    },
}

#[derive(Debug)]
pub enum Outcome {
    Job(Box<FineTuneJob>),
    Page(Vec<u8>),
}

impl Dispatch<'_> {
    pub fn run(self) -> Result<Outcome, FineTuneError> {
        let Self { connection, call } = self;
        Ok(match call {
            Call::Create(data) => Outcome::Job(Box::new(request_fine_tuning_job(
                connection,
                &JobRequest::Create(data),
            )?)),
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
