use std::sync::{Arc, PoisonError};

use betterlite_logging::logging_success::ports::StandardPayloadBuilder;
use betterlite_logging::logging_success::types::SuccessResult;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_transcribe::policy::cost::{started_transcription_job, transcribe_max_job_cost};
use betterlite_transcribe::records::StartedJob;
use chrono::{DateTime, FixedOffset};
use http::HeaderMap;
use rust_decimal::Decimal;
use tokio::sync::Semaphore;
use url::Url;

use super::{
    JobPricer, JobPricingRequest, ScheduleInput, TRANSCRIBE_PRICED_MODEL, TranscribeHandlerInput,
    TranscribeRates, transcribe_passthrough_handler,
};

pub(super) struct PricedJob {
    request_headers: HeaderMap,
    request_url: Url,
    started_job: StartedJob,
    logging_obj: Arc<StreamLoggingObject>,
    url_route: String,
    result: String,
    start_time: DateTime<FixedOffset>,
    end_time: DateTime<FixedOffset>,
    cache_hit: bool,
    request_body: JsonMap,
    log: Arc<dyn StreamSuccessDispatch>,
    kwargs: PassThroughLoggingKwargs,
    payload_builder: Arc<dyn StandardPayloadBuilder>,
}

impl PricedJob {
    #[must_use]
    pub(super) fn new(input: ScheduleInput<'_>) -> Self {
        Self {
            request_headers: input.request_headers.clone(),
            request_url: input.request_url.clone(),
            started_job: started_job(input.response_body),
            logging_obj: input.logging_obj,
            url_route: input.url_route.to_owned(),
            result: input.result.to_owned(),
            start_time: input.start_time,
            end_time: input.end_time,
            cache_hit: input.cache_hit,
            request_body: input.request_body.clone(),
            log: input.log,
            kwargs: input.kwargs,
            payload_builder: input.payload_builder,
        }
    }
}

pub(super) struct JobWorker {
    pub(super) job_pricer: Arc<dyn JobPricer>,
    pub(super) rates: Arc<dyn TranscribeRates>,
    pub(super) pending: Arc<Semaphore>,
}

impl JobWorker {
    pub(super) async fn run(self, job: PricedJob) {
        let Some(rate) = self.rates.cost_per_second() else {
            tracing::error!(
                "{TRANSCRIBE_PRICED_MODEL} left the model cost map, spend not recorded"
            );
            return;
        };
        let job_name = job_name(&job.request_body);
        let Some(region) = job.request_url.host_str().and_then(region_name) else {
            tracing::error!(
                url = %job.request_url,
                "the Transcribe request URL has no usable region, spend not recorded"
            );
            return;
        };
        let permit = Arc::clone(&self.pending).try_acquire_owned().ok();
        let cost = if permit.is_some() {
            self.priced(&job, &job_name, region, rate).await
        } else {
            full_queue_cost(&job_name, rate)
        };
        let Some(charged) = cost else {
            return;
        };
        dispatch(job, charged).await;
        drop(permit);
    }

    async fn priced(
        &self,
        job: &PricedJob,
        job_name: &str,
        region: &str,
        rate: Decimal,
    ) -> Option<Decimal> {
        let request = JobPricingRequest {
            job_name,
            aws_region_name: region,
            cost_per_second: rate,
            started_job: &job.started_job,
        };
        match self.job_pricer.price_job(request).await {
            Ok(cost) => Some(cost),
            Err(error) => {
                tracing::error!(
                    %error,
                    job = %job_name,
                    "Pricing the Transcribe job failed, spend not recorded"
                );
                None
            }
        }
    }
}

fn full_queue_cost(job_name: &str, rate: Decimal) -> Option<Decimal> {
    tracing::warn!("Transcribe pricing queue is full, charging maximum for job {job_name}");
    match transcribe_max_job_cost(rate) {
        Ok(cost) => Some(cost),
        Err(error) => {
            tracing::error!(
                %error,
                "the Transcribe maximum job cost is out of range, spend not recorded"
            );
            None
        }
    }
}

async fn dispatch(job: PricedJob, cost: Decimal) {
    let outcome = transcribe_passthrough_handler(TranscribeHandlerInput {
        logging_obj: &job.logging_obj,
        url_route: &job.url_route,
        result: &job.result,
        start_time: job.start_time,
        end_time: job.end_time,
        cache_hit: job.cache_hit,
        request_headers: &job.request_headers,
        request_body: &job.request_body,
        response_cost: cost,
        kwargs: job.kwargs,
        payload_builder: job.payload_builder.as_ref(),
    });
    let result = outcome
        .result
        .map_or_else(|| SuccessResult::Other(Json::Null), SuccessResult::from);
    dispatch_output(SpendDispatch {
        logging_obj: job.logging_obj,
        log: job.log,
        result,
        kwargs: outcome.kwargs,
        start_time: job.start_time,
        end_time: job.end_time,
        cache_hit: job.cache_hit,
    })
    .await;
}

struct SpendDispatch {
    logging_obj: Arc<StreamLoggingObject>,
    log: Arc<dyn StreamSuccessDispatch>,
    result: SuccessResult,
    kwargs: PassThroughLoggingKwargs,
    start_time: DateTime<FixedOffset>,
    end_time: DateTime<FixedOffset>,
    cache_hit: bool,
}

async fn dispatch_output(dispatch: SpendDispatch) {
    let taken = dispatch
        .logging_obj
        .slot
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    let Some(mut state) = taken else {
        tracing::error!("the Transcribe logging state was taken before the spend dispatch");
        return;
    };
    dispatch
        .log
        .dispatch_success(StreamSuccessCall {
            state: &mut state.logging,
            correlation: &mut state.correlation,
            result: dispatch.result,
            start_time: dispatch.start_time,
            end_time: dispatch.end_time,
            cache_hit: dispatch.cache_hit,
            kwargs: &dispatch.kwargs,
        })
        .await;
    *dispatch
        .logging_obj
        .slot
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(state);
}

fn started_job(response_body: Option<&JsonMap>) -> StartedJob {
    let body = response_body?;
    let text = serde_json::to_string(body).ok()?;
    started_transcription_job(&text)
}

fn job_name(request_body: &JsonMap) -> String {
    request_body
        .get("TranscriptionJobName")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_owned()
}

pub(super) fn region_name(host: &str) -> Option<&str> {
    let region = host.split('.').nth(1)?;
    let allowed = !region.is_empty()
        && region
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    allowed.then_some(region)
}
