use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct TranscriptionJobTag {
    pub key: Option<String>,
    pub value: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct TranscriptionMedia {
    pub media_file_uri: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct TranscriptionJobRecord {
    pub transcription_job_status: Option<String>,
    pub creation_time: Option<f64>,
    pub media: Option<TranscriptionMedia>,
    #[serde(default)]
    pub tags: Vec<TranscriptionJobTag>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct TranscriptionJobResponse {
    pub transcription_job: Option<TranscriptionJobRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranscribeRefusal {
    pub status_code: u16,
    pub detail: String,
}

pub type StartedJob = Option<TranscriptionJobRecord>;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetTranscriptionJobRequest {
    pub transcription_job_name: String,
}

pub type JobLookupFuture<'lookup> = Pin<
    Box<dyn Future<Output = Result<TranscriptionJobResponse, JobLookupError>> + Send + 'lookup>,
>;

pub trait JobLookup: Send + Sync {
    fn get_transcription_job<'lookup>(
        &'lookup self,
        request: &'lookup GetTranscriptionJobRequest,
    ) -> JobLookupFuture<'lookup>;
}

#[derive(Debug, thiserror::Error)]
pub enum JobLookupError {
    #[error("Transcribe job lookup failed")]
    Unavailable,
}

mod tests;
