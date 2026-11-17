#![cfg(test)]

mod support;

use std::sync::{Arc, PoisonError};

use betterlite_core::api::media_relay_logging::spend::decimal_json;
use betterlite_logging::passthrough_logging_state::response::{
    PassThroughResponse, PassThroughResponseObject,
};
use betterlite_models::decisions::Json;
use betterlite_transcribe::policy::cost::transcribe_max_job_cost;
use betterlite_transcribe::records::StartedJob;
use http::{HeaderMap, HeaderValue};
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;
use url::Url;

use super::schedule::region_name;
use super::{
    JobPricingRequest, MAX_PENDING_PRICING_JOBS, TranscribeLogger, is_priced_job_start,
    operation_from_target, transcribe_passthrough_handler,
};
use support::{
    FixedPricer, Job, OTHER_TARGET, RESULT_TEXT, RecordingLive, START_TARGET, capture_log, details,
    end_time, entry, failing_fixture, field, handler_fixture, headers, headers_with, job_record,
    json, json_map, log_text, logged_model, logged_provider, logger_with, price, rate, rates,
    start_time,
};

#[test]
fn operation_from_target_keeps_the_text_after_the_last_dot() {
    assert_eq!(
        operation_from_target(Some("Transcribe.StartTranscriptionJob")),
        "StartTranscriptionJob"
    );
}

#[test]
fn operation_from_target_cuts_nested_dots_at_the_last_one() {
    assert_eq!(
        operation_from_target(Some("com.amazon.transcribe.ListJobs")),
        "ListJobs"
    );
}

#[test]
fn operation_from_target_missing_target_is_empty() {
    assert_eq!(operation_from_target(None), "");
}

#[test]
fn operation_from_target_without_a_dot_comes_back_whole() {
    assert_eq!(
        operation_from_target(Some("StartTranscriptionJob")),
        "StartTranscriptionJob"
    );
}

#[test]
fn operation_from_target_trailing_dot_is_empty() {
    assert_eq!(operation_from_target(Some("Transcribe.")), "");
}

#[test]
fn operation_priced_start_reads_the_upstream_request_header() {
    assert!(is_priced_job_start(&headers(START_TARGET)));
}

#[test]
fn operation_priced_start_looks_up_the_header_case_insensitively() {
    assert!(is_priced_job_start(&headers_with(
        "X-Amz-Target",
        START_TARGET
    )));
}

#[test]
fn operation_priced_start_rejects_another_operation() {
    assert!(!is_priced_job_start(&headers(OTHER_TARGET)));
}

#[test]
fn operation_priced_start_rejects_a_missing_header() {
    assert!(!is_priced_job_start(&HeaderMap::new()));
}

#[test]
fn operation_priced_start_rejects_invalid_header_text() {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-amz-target",
        HeaderValue::from_bytes(&[0xff]).expect("a raw value"),
    );
    assert!(!is_priced_job_start(&headers));
}

#[test]
fn operation_priced_start_matches_the_operation_alone() {
    assert_eq!(
        operation_from_target(Some(START_TARGET)),
        "StartTranscriptionJob"
    );
    assert_ne!(
        operation_from_target(Some(OTHER_TARGET)),
        "StartTranscriptionJob"
    );
}

#[test]
fn handler_returns_the_result_text_as_a_pass_through_response_object() {
    let fixture = handler_fixture();
    let outcome = transcribe_passthrough_handler(fixture.input());
    let expected = Json::Text(RESULT_TEXT.to_owned());
    assert_eq!(
        outcome.result,
        Some(PassThroughResponse::Standard(PassThroughResponseObject {
            response: expected,
        }))
    );
}

#[test]
fn handler_writes_the_model_provider_and_cost_into_the_kwargs() {
    let fixture = handler_fixture();
    let outcome = transcribe_passthrough_handler(fixture.input());
    assert_eq!(
        entry(&outcome.kwargs, "model"),
        &json("\"transcribe/StartTranscriptionJob\"")
    );
    assert_eq!(
        entry(&outcome.kwargs, "custom_llm_provider"),
        &json("\"transcribe\"")
    );
    assert_eq!(entry(&outcome.kwargs, "response_cost"), &json("2.5"));
    assert_eq!(entry(&outcome.kwargs, "existing"), &Json::Bool(true));
}

#[test]
fn handler_writes_the_model_call_details() {
    let fixture = handler_fixture();
    let outcome = transcribe_passthrough_handler(fixture.input());
    assert!(outcome.result.is_some());
    let written = details(&fixture.logging_obj);
    assert_eq!(
        written.get("model"),
        Some(&json("\"transcribe/StartTranscriptionJob\""))
    );
    assert_eq!(
        written.get("custom_llm_provider"),
        Some(&json("\"transcribe\""))
    );
    assert_eq!(written.get("response_cost"), Some(&json("2.5")));
    assert_eq!(
        logged_model(&fixture.logging_obj),
        "transcribe/StartTranscriptionJob"
    );
    assert_eq!(
        logged_provider(&fixture.logging_obj).as_deref(),
        Some("transcribe")
    );
}

#[test]
fn handler_builds_the_payload_after_the_model_call_details_write() {
    let fixture = handler_fixture();
    let outcome = transcribe_passthrough_handler(fixture.input());
    assert!(outcome.result.is_some());
    let calls = fixture.builder.calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.model, "transcribe/StartTranscriptionJob");
    assert_eq!(call.provider.as_deref(), Some("transcribe"));
    assert_eq!(call.details.get("response_cost"), Some(&json("2.5")));
    assert_eq!(
        call.details.get("custom_llm_provider"),
        Some(&json("\"transcribe\""))
    );
    assert_eq!(call.start_time, start_time());
    assert_eq!(call.end_time, end_time());
    let response = Json::Text(RESULT_TEXT.to_owned());
    assert_eq!(field(&call.response, "response"), &response);
}

#[test]
fn handler_includes_the_standard_logging_object() {
    let fixture = handler_fixture();
    let outcome = transcribe_passthrough_handler(fixture.input());
    let payload = entry(&outcome.kwargs, "standard_logging_object");
    assert_eq!(field(payload, "id"), &json("\"chatcmpl-1\""));
    assert_eq!(field(payload, "status"), &json("\"success\""));
    assert_eq!(
        field(payload, "model"),
        &json("\"transcribe/StartTranscriptionJob\"")
    );
}

#[test]
fn handler_model_without_the_target_header_is_the_provider_and_a_slash() {
    let mut fixture = handler_fixture();
    fixture.headers = HeaderMap::new();
    let outcome = transcribe_passthrough_handler(fixture.input());
    assert_eq!(entry(&outcome.kwargs, "model"), &json("\"transcribe/\""));
}

#[test]
fn fallback_returns_the_original_kwargs() {
    let fixture = failing_fixture();
    let original = fixture.kwargs.clone();
    let outcome = transcribe_passthrough_handler(fixture.input());
    assert_eq!(outcome.kwargs, original);
}

#[test]
fn fallback_keeps_the_model_call_details_write() {
    let fixture = failing_fixture();
    let outcome = transcribe_passthrough_handler(fixture.input());
    assert!(outcome.result.is_some());
    let written = details(&fixture.logging_obj);
    assert_eq!(
        written.get("model"),
        Some(&json("\"transcribe/StartTranscriptionJob\""))
    );
    assert_eq!(
        written.get("custom_llm_provider"),
        Some(&json("\"transcribe\""))
    );
    assert_eq!(written.get("response_cost"), Some(&json("2.5")));
}

#[test]
fn fallback_still_returns_the_result_text() {
    let fixture = failing_fixture();
    let outcome = transcribe_passthrough_handler(fixture.input());
    let expected = Json::Text(RESULT_TEXT.to_owned());
    assert_eq!(
        outcome.result,
        Some(PassThroughResponse::Standard(PassThroughResponseObject {
            response: expected,
        }))
    );
}

#[test]
fn fallback_logs_the_error() {
    let (log, _guard) = capture_log();
    let fixture = failing_fixture();
    let outcome = transcribe_passthrough_handler(fixture.input());
    assert!(outcome.result.is_some());
    assert!(log_text(&log).contains(
        "Error in Amazon Transcribe passthrough logging handler: the standard logging payload could not be built"
    ));
}

#[test]
fn fallback_when_the_state_was_already_taken() {
    let fixture = handler_fixture();
    let taken = fixture
        .logging_obj
        .slot
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    assert!(taken.is_some());
    let original = fixture.kwargs.clone();
    let outcome = transcribe_passthrough_handler(fixture.input());
    assert_eq!(outcome.kwargs, original);
    assert!(fixture.builder.calls().is_empty());
}

#[tokio::test]
async fn schedule_prices_with_the_job_name_region_rate_and_started_job() {
    let job = Job::new();
    let pricer = FixedPricer::pricing(price());
    let logger = logger_with(Arc::clone(&pricer), Some(rate()), 10);
    job.run(&logger).await;
    let calls = pricer.calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.job_name, "job-1");
    assert_eq!(call.region, "us-east-1");
    assert_eq!(call.cost_per_second, rate());
    assert_eq!(call.started_job, Some(job_record()));
}

#[tokio::test]
async fn schedule_region_is_the_second_host_label() {
    let mut job = Job::new();
    job.request_url =
        Url::parse("https://transcribe.cn-north-1.amazonaws.com.cn/").expect("a valid URL");
    let pricer = FixedPricer::pricing(price());
    let logger = logger_with(Arc::clone(&pricer), Some(rate()), 10);
    job.run(&logger).await;
    assert_eq!(pricer.calls()[0].region, "cn-north-1");
}

#[tokio::test]
async fn schedule_job_name_that_is_not_a_string_is_empty() {
    let mut job = Job::new();
    job.request_body = json_map(r#"{"TranscriptionJobName":7}"#);
    let pricer = FixedPricer::pricing(price());
    let logger = logger_with(Arc::clone(&pricer), Some(rate()), 10);
    job.run(&logger).await;
    assert_eq!(pricer.calls()[0].job_name, "");
}

#[tokio::test]
async fn schedule_missing_job_name_is_empty() {
    let mut job = Job::new();
    job.request_body = json_map("{}");
    let pricer = FixedPricer::pricing(price());
    let logger = logger_with(Arc::clone(&pricer), Some(rate()), 10);
    job.run(&logger).await;
    assert_eq!(pricer.calls()[0].job_name, "");
}

#[tokio::test]
async fn schedule_missing_response_body_has_no_started_job() {
    let mut job = Job::new();
    job.response_body = None;
    let pricer = FixedPricer::pricing(price());
    let logger = logger_with(Arc::clone(&pricer), Some(rate()), 10);
    job.run(&logger).await;
    assert_eq!(pricer.calls()[0].started_job, None);
}

#[tokio::test]
async fn schedule_dispatches_the_priced_cost_and_the_received_cache_hit() {
    let job = Job::new();
    let pricer = FixedPricer::pricing(price());
    let logger = logger_with(Arc::clone(&pricer), Some(rate()), 10);
    job.run(&logger).await;
    let calls = job.log.calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert!(call.cache_hit);
    assert_eq!(entry(&call.kwargs, "response_cost"), &decimal_json(price()));
    assert_eq!(
        entry(&call.kwargs, "model"),
        &json("\"transcribe/StartTranscriptionJob\"")
    );
    assert_eq!(call.start_time, start_time());
    assert_eq!(call.end_time, end_time());
}

#[tokio::test]
async fn schedule_dispatches_the_handler_result_and_payload() {
    let job = Job::new();
    let pricer = FixedPricer::pricing(price());
    let logger = logger_with(Arc::clone(&pricer), Some(rate()), 10);
    job.run(&logger).await;
    let calls = job.log.calls();
    assert_eq!(calls.len(), 1);
    let expected = Json::Text(RESULT_TEXT.to_owned());
    assert_eq!(field(&calls[0].result, "response"), &expected);
    let payload = entry(&calls[0].kwargs, "standard_logging_object");
    assert_eq!(field(payload, "id"), &json("\"chatcmpl-1\""));
    assert_eq!(entry(&calls[0].kwargs, "existing"), &Json::Bool(true));
}

#[tokio::test]
async fn schedule_restores_the_logging_state_after_the_dispatch() {
    let job = Job::new();
    let pricer = FixedPricer::pricing(price());
    let logger = logger_with(Arc::clone(&pricer), Some(rate()), 10);
    job.run(&logger).await;
    assert_eq!(
        logged_model(&job.logging_obj),
        "transcribe/StartTranscriptionJob"
    );
}

#[tokio::test]
async fn failure_missing_rate_logs_and_dispatches_nothing() {
    let (log, _guard) = capture_log();
    let job = Job::new();
    let pricer = FixedPricer::pricing(price());
    let logger = logger_with(Arc::clone(&pricer), None, 10);
    job.run(&logger).await;
    assert!(job.log.calls().is_empty());
    assert!(pricer.calls().is_empty());
    assert!(
        log_text(&log).contains(
            "transcribe/StartTranscriptionJob left the model cost map, spend not recorded"
        )
    );
}

#[test]
fn failure_region_guards_reject_hosts_without_a_valid_second_label() {
    assert_eq!(region_name("transcribe"), None);
    assert_eq!(region_name("transcribe..amazonaws.com"), None);
    assert_eq!(region_name("transcribe.US-EAST-1.amazonaws.com"), None);
    assert_eq!(region_name("transcribe.us_east_1.amazonaws.com"), None);
    assert_eq!(
        region_name("transcribe.us-east-1.amazonaws.com"),
        Some("us-east-1")
    );
    assert_eq!(
        region_name("transcribe.cn-north-1.amazonaws.com.cn"),
        Some("cn-north-1")
    );
}

#[tokio::test]
async fn failure_region_without_a_second_label_dispatches_nothing() {
    let mut job = Job::new();
    job.request_url = Url::parse("https://transcribe/").expect("a valid URL");
    let pricer = FixedPricer::pricing(price());
    let logger = logger_with(Arc::clone(&pricer), Some(rate()), 10);
    job.run(&logger).await;
    assert!(job.log.calls().is_empty());
    assert!(pricer.calls().is_empty());
}

#[tokio::test]
async fn failure_pricer_error_dispatches_nothing() {
    let job = Job::new();
    let pricer = FixedPricer::failing();
    let logger = logger_with(Arc::clone(&pricer), Some(rate()), 10);
    job.run(&logger).await;
    assert!(job.log.calls().is_empty());
    assert_eq!(pricer.calls().len(), 1);
}

#[tokio::test]
async fn failure_taken_state_dispatches_nothing_and_ends_the_task() {
    let job = Job::new();
    let taken = job
        .logging_obj
        .slot
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    assert!(taken.is_some());
    let pricer = FixedPricer::pricing(price());
    let logger = logger_with(Arc::clone(&pricer), Some(rate()), 10);
    job.run(&logger).await;
    assert!(job.log.calls().is_empty());
    assert_eq!(pricer.calls().len(), 1);
}

#[tokio::test]
async fn limits_full_queue_charges_the_maximum_without_pricing() {
    let (log, _guard) = capture_log();
    let job = Job::new();
    let pricer = FixedPricer::pricing(price());
    let logger = logger_with(Arc::clone(&pricer), Some(rate()), 1);
    let held = Arc::clone(&logger.pending)
        .try_acquire_owned()
        .expect("a permit is free");
    job.run(&logger).await;
    assert!(pricer.calls().is_empty());
    let calls = job.log.calls();
    assert_eq!(calls.len(), 1);
    let maximum = transcribe_max_job_cost(rate()).expect("a valid maximum");
    assert_eq!(
        entry(&calls[0].kwargs, "response_cost"),
        &decimal_json(maximum)
    );
    assert!(
        log_text(&log).contains("Transcribe pricing queue is full, charging maximum for job job-1")
    );
    drop(held);
}

#[tokio::test]
async fn limits_permit_is_released_after_the_task_ends() {
    let first = Job::new();
    let second = Job::new();
    let pricer = FixedPricer::pricing(price());
    let logger = logger_with(Arc::clone(&pricer), Some(rate()), 1);
    first.run(&logger).await;
    second.run(&logger).await;
    assert_eq!(pricer.calls().len(), 2);
    assert_eq!(first.log.calls().len(), 1);
    assert_eq!(second.log.calls().len(), 1);
    assert_eq!(logger.pending.available_permits(), 1);
}

#[tokio::test]
async fn wiring_production_shares_one_download_semaphore_across_pricing_calls() {
    let live = RecordingLive::default();
    let logger =
        TranscribeLogger::production(live.clone(), rates(Some(rate())), TaskTracker::new());
    let started_job: StartedJob = None;
    let first = logger
        .job_pricer
        .price_job(JobPricingRequest {
            job_name: "job-1",
            aws_region_name: "us-east-1",
            cost_per_second: rate(),
            started_job: &started_job,
        })
        .await
        .expect("the production pricer prices");
    let second = logger
        .job_pricer
        .price_job(JobPricingRequest {
            job_name: "job-2",
            aws_region_name: "eu-west-1",
            cost_per_second: rate(),
            started_job: &started_job,
        })
        .await
        .expect("the production pricer prices");
    assert_eq!(first, Decimal::ONE);
    assert_eq!(second, Decimal::ONE);
    let slots = live
        .slots
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(slots.len(), 2);
    assert!(Arc::ptr_eq(&slots[0], &slots[1]));
    assert_eq!(slots[0].available_permits(), 1);
    assert_eq!(logger.pending.available_permits(), MAX_PENDING_PRICING_JOBS);
}
