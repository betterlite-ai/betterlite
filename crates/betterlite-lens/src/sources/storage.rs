use std::error::Error as StdError;
use std::future::Future;

use rust_decimal::Decimal;

use crate::models::execution::ExecutionSource;
use crate::models::settings::Source;
use crate::sources::parameters::Parameters;

pub trait Storage {
    type Error: StdError + Send + Sync + 'static;

    fn lens_availability(
        &self,
        parameters: Parameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send;

    fn lens_agents(
        &self,
        parameters: Parameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send;

    fn lens_sample(
        &self,
        parameters: SampleParameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send;

    fn lens_content(
        &self,
        parameters: ContentParameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send;

    fn lens_evidence(
        &self,
        parameters: EvidenceParameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SampleParameters {
    pub base: Parameters,
    pub source: Source,
    pub start: i64,
    pub end: i64,
    pub service: String,
    pub agent_name: String,
    pub limit: usize,
    pub offset: i64,
    pub after: String,
    pub sample_percent: Decimal,
    pub sample_cap: u64,
    pub preview: bool,
    pub selected_team: String,
    pub execution_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentParameters {
    pub base: Parameters,
    pub source: ExecutionSource,
    pub id: String,
    pub trace_ref: String,
    pub record_team: String,
    pub cursor: String,
    pub offset: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvidenceParameters {
    pub base: Parameters,
    pub source: ExecutionSource,
    pub id: String,
    pub trace_ref: String,
    pub record_team: String,
    pub span: String,
    pub quote: String,
}
