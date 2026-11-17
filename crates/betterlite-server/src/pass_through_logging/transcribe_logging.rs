mod error;
mod handler;
mod schedule;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use betterlite_logging::logging_success::ports::StandardPayloadBuilder;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamSuccessDispatch,
};
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_transcribe::live::{LiveDeps, price_transcription_job_live};
use betterlite_transcribe::media_pricing::PricingError;
use betterlite_transcribe::records::StartedJob;
use chrono::{DateTime, FixedOffset};
use http::HeaderMap;
use rust_decimal::Decimal;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tokio_util::task::TaskTracker;
use url::Url;

pub const TRANSCRIBE_CUSTOM_LLM_PROVIDER: &str = "transcribe";
pub const TRANSCRIBE_PRICED_MODEL: &str = "transcribe/StartTranscriptionJob";
pub const TRANSCRIBE_PRICED_OPERATION: &str = "StartTranscriptionJob";
// ast-grep-ignore: no-long-names
pub const TRANSCRIBE_MEDIA_DOWNLOAD_CONCURRENCY: usize = 1;
pub const MAX_PENDING_PRICING_JOBS: usize = 10_000;

pub struct TranscribeHandlerInput<'input> {
    pub logging_obj: &'input StreamLoggingObject,
    pub url_route: &'input str,
    pub result: &'input str,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub cache_hit: bool,
    pub request_headers: &'input HeaderMap,
    pub request_body: &'input JsonMap,
    pub response_cost: Decimal,
    pub kwargs: PassThroughLoggingKwargs,
    pub payload_builder: &'input dyn StandardPayloadBuilder,
}

#[must_use]
// ast-grep-ignore: no-one-line-functions - the fixed public predicate B3607 calls
pub fn is_priced_job_start(request_headers: &HeaderMap) -> bool {
    operation_from_headers(request_headers) == TRANSCRIBE_PRICED_OPERATION
}

#[must_use]
pub fn operation_from_target(target: Option<&str>) -> &str {
    let Some(value) = target else {
        return "";
    };
    value
        .rsplit_once('.')
        .map_or(value, |(_prefix, operation)| operation)
}

fn operation_from_headers(request_headers: &HeaderMap) -> &str {
    let Some(value) = request_headers.get("x-amz-target") else {
        return "";
    };
    operation_from_target(value.to_str().ok())
}

#[must_use]
// ast-grep-ignore: no-one-line-functions - the fixed public handler name B3606 calls
pub fn transcribe_passthrough_handler(input: TranscribeHandlerInput<'_>) -> HandlerOutcome {
    handler::outcome(input)
}

pub type JobPricingFuture<'job> =
    Pin<Box<dyn Future<Output = Result<Decimal, PricingError>> + Send + 'job>>;

pub struct JobPricingRequest<'job> {
    pub job_name: &'job str,
    pub aws_region_name: &'job str,
    pub cost_per_second: Decimal,
    pub started_job: &'job StartedJob,
}

pub trait JobPricer: Send + Sync {
    fn price_job<'job>(&'job self, request: JobPricingRequest<'job>) -> JobPricingFuture<'job>;
}

pub trait LivePricing: Send + Sync {
    fn price<'call>(
        &'call self,
        request: JobPricingRequest<'call>,
        download_slots: Arc<Semaphore>,
    ) -> JobPricingFuture<'call>;
}

impl LivePricing for LiveDeps<'_> {
    fn price<'call>(
        &'call self,
        request: JobPricingRequest<'call>,
        download_slots: Arc<Semaphore>,
    ) -> JobPricingFuture<'call> {
        let deps = LiveDeps {
            resolver: self.resolver,
            http: self.http.clone(),
            region: request.aws_region_name.to_owned(),
            download_slots,
        };
        Box::pin(async move {
            price_transcription_job_live(
                request.job_name,
                request.cost_per_second,
                request.started_job.clone(),
                deps,
            )
            .await
        })
    }
}

pub struct LiveJobPricer<D = LiveDeps<'static>> {
    deps: D,
    download_slots: Arc<Semaphore>,
}

impl<D> LiveJobPricer<D> {
    #[must_use]
    pub const fn new(deps: D, download_slots: Arc<Semaphore>) -> Self {
        Self {
            deps,
            download_slots,
        }
    }
}

impl<D> JobPricer for LiveJobPricer<D>
where
    D: LivePricing,
{
    fn price_job<'job>(&'job self, request: JobPricingRequest<'job>) -> JobPricingFuture<'job> {
        self.deps.price(request, Arc::clone(&self.download_slots))
    }
}

pub trait TranscribeRates: Send + Sync {
    fn cost_per_second(&self) -> Option<Decimal>;
}

pub struct ScheduleInput<'input> {
    pub request_headers: &'input HeaderMap,
    pub request_url: &'input Url,
    pub response_body: Option<&'input JsonMap>,
    pub logging_obj: Arc<StreamLoggingObject>,
    pub url_route: &'input str,
    pub result: &'input str,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub cache_hit: bool,
    pub request_body: &'input JsonMap,
    pub log: Arc<dyn StreamSuccessDispatch>,
    pub kwargs: PassThroughLoggingKwargs,
    pub payload_builder: Arc<dyn StandardPayloadBuilder>,
}

pub struct TranscribeLogger {
    job_pricer: Arc<dyn JobPricer>,
    rates: Arc<dyn TranscribeRates>,
    pending: Arc<Semaphore>,
    tasks: TaskTracker,
}

impl TranscribeLogger {
    #[must_use]
    pub fn new(
        job_pricer: Arc<dyn JobPricer>,
        rates: Arc<dyn TranscribeRates>,
        max_pending: usize,
        tasks: TaskTracker,
    ) -> Self {
        Self {
            job_pricer,
            rates,
            pending: Arc::new(Semaphore::new(max_pending)),
            tasks,
        }
    }

    #[must_use]
    pub fn production<P>(deps: P, rates: Arc<dyn TranscribeRates>, tasks: TaskTracker) -> Self
    where
        P: LivePricing + 'static,
    {
        let download_slots = Arc::new(Semaphore::new(
            // ast-grep-ignore: no-long-names
            TRANSCRIBE_MEDIA_DOWNLOAD_CONCURRENCY,
        ));
        let pricer = LiveJobPricer::new(deps, Arc::clone(&download_slots));
        Self::new(Arc::new(pricer), rates, MAX_PENDING_PRICING_JOBS, tasks)
    }

    #[must_use]
    pub fn schedule_priced_job_logging(&self, input: ScheduleInput<'_>) -> JoinHandle<()> {
        let job = schedule::PricedJob::new(input);
        let worker = schedule::JobWorker {
            job_pricer: Arc::clone(&self.job_pricer),
            rates: Arc::clone(&self.rates),
            pending: Arc::clone(&self.pending),
        };
        self.tasks.spawn(worker.run(job))
    }
}

mod tests;
