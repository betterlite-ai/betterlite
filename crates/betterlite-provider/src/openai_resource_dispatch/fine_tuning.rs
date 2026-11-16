use betterlite_openai::fine_tuning::client::Connection;
use betterlite_openai::fine_tuning::error::FineTuneError;
use betterlite_openai::fine_tuning::job::{FineTuneCreate, FineTuneJob};
use betterlite_openai::fine_tuning::{JobRequest, list_fine_tuning_jobs, request_fine_tuning_job};

pub struct Dispatch<'a> {
    pub connection: Connection<'a>,
    pub call: Call<'a>,
}

// B518's four lifecycle calls; list hands back the SDK page bytes it decoded nothing in
pub enum Call<'a> {
    Create(&'a FineTuneCreate),
    Retrieve(&'a str),
    Cancel(&'a str),
    List {
        after: Option<&'a str>,
        limit: Option<i64>,
    },
}

#[derive(Debug)]
pub enum Outcome {
    Job(Box<FineTuneJob>),
    Page(Vec<u8>),
}

impl Dispatch<'_> {
    #[must_use]
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
