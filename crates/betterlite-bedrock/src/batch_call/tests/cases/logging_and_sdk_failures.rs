use std::collections::BTreeMap;

use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::BatchObject;

use crate::batch_call::cancel::{CancelBatchCall, cancel_batch};
use crate::batch_call::error::BatchCallError;
use crate::batch_call::logging::{CallEvent, CallLogger};
use crate::batch_call::status::{JobStatusCall, retrieve_model_job_status};

use super::super::fixture::{
    Factory, JOB_ARN, JOB_ID, REGION, Reply, Source, answer, auth_parameters, credentials, field,
    logged, now, runtime, text,
};

const LOGGED_BODY: &str = r#"{"jobArn":"arn:aws:bedrock:eu-west-1:123456789012:model-invocation-job/job-1","jobName":"name","modelId":"model","status":"InProgress","totalRecordCount":2,"successRecordCount":1,"errorRecordCount":1}"#;
const API_BASE: &str = "https://bedrock.eu-west-1.amazonaws.com/model-invocation-job/job-1";

pub(super) fn logging_and_sdk_failures() {
    logs_the_call();
    bare_id_api_base();
    transport_failure_propagates();
    credential_failure_precedes_logging();
    stop_failure_propagates();
    logging_failure_propagates();
}

fn capture(batch_id: &str) -> (Result<BatchObject, BatchCallError>, Vec<Json>) {
    let source = Source::new(credentials());
    let factory = Factory::new(vec![answer(200, LOGGED_BODY)]);
    let kwargs = auth_parameters();
    let call = JobStatusCall {
        batch_id,
        aws_region_name: Some(REGION),
        kwargs: &kwargs,
        now: now(),
    };
    logged(|logger| runtime().block_on(retrieve_model_job_status(call, &source, &factory, logger)))
}

fn logs_the_call() {
    let (result, snapshots) = capture(JOB_ARN);
    result.expect("the lookup succeeds");
    assert_eq!(snapshots.len(), 2);
    assert_pre(&snapshots[0]);
    assert_post(&snapshots[1]);
}

fn assert_pre(snapshot: &Json) {
    assert_eq!(text(field(snapshot, "log_event_type")), "pre_api_call");
    assert_eq!(text(field(snapshot, "input")), JOB_ARN);
    let args = field(snapshot, "additional_args");
    assert_eq!(text(field(args, "api_base")), API_BASE);
    let complete = field(args, "complete_input_dict");
    assert_eq!(text(field(complete, "jobIdentifier")), JOB_ARN);
    let params = field(snapshot, "litellm_params");
    assert_eq!(text(field(params, "api_base")), API_BASE);
}

fn assert_post(snapshot: &Json) {
    assert_eq!(text(field(snapshot, "log_event_type")), "post_api_call");
    assert_eq!(text(field(snapshot, "input")), JOB_ARN);
    let args = field(snapshot, "additional_args");
    assert_eq!(field(args, "api_base"), &Json::Null);
    let complete = field(args, "complete_input_dict");
    assert_eq!(text(field(complete, "jobIdentifier")), JOB_ARN);
    let recorded: Json = serde_json::from_str(text(field(snapshot, "original_response")))
        .expect("the logged response is JSON");
    let expected: Json = serde_json::from_str(LOGGED_BODY).expect("the fixture body is JSON");
    assert_eq!(recorded, expected);
}

fn bare_id_api_base() {
    let (result, snapshots) = capture(JOB_ID);
    result.expect("the lookup succeeds");
    let args = field(&snapshots[0], "additional_args");
    assert_eq!(text(field(args, "api_base")), API_BASE);
}

fn transport_failure_propagates() {
    let source = Source::new(credentials());
    let factory = Factory::new(vec![Reply::Fail]);
    let kwargs = BTreeMap::new();
    let call = JobStatusCall {
        batch_id: JOB_ID,
        aws_region_name: Some(REGION),
        kwargs: &kwargs,
        now: now(),
    };
    let (result, snapshots) = logged(|logger| {
        runtime().block_on(retrieve_model_job_status(call, &source, &factory, logger))
    });
    let error = result.expect_err("a failed transport propagates");
    assert!(matches!(error, BatchCallError::Transport(_)));
    assert_eq!(snapshots.len(), 1);
    assert_eq!(factory.requests().len(), 1);
}

fn credential_failure_precedes_logging() {
    let source = Source::failing();
    let factory = Factory::new(Vec::new());
    let kwargs = BTreeMap::new();
    let call = JobStatusCall {
        batch_id: JOB_ID,
        aws_region_name: Some(REGION),
        kwargs: &kwargs,
        now: now(),
    };
    let (result, snapshots) = logged(|logger| {
        runtime().block_on(retrieve_model_job_status(call, &source, &factory, logger))
    });
    let error = result.expect_err("a credential failure propagates");
    assert!(matches!(error, BatchCallError::Credentials(_)));
    assert!(snapshots.is_empty());
    assert!(factory.configs().is_empty());
}

fn stop_failure_propagates() {
    let source = Source::new(credentials());
    let factory = Factory::new(vec![Reply::Fail]);
    let kwargs = BTreeMap::new();
    let call = CancelBatchCall {
        batch_id: JOB_ID,
        aws_region_name: Some(REGION),
        kwargs: &kwargs,
        now: now(),
    };
    let error = runtime()
        .block_on(cancel_batch(call, &source, &factory, None))
        .expect_err("a failed stop propagates");
    assert!(matches!(error, BatchCallError::Transport(_)));
    assert_eq!(factory.requests().len(), 1);
}

fn logging_failure_propagates() {
    let source = Source::new(credentials());
    let factory = Factory::new(vec![answer(200, LOGGED_BODY)]);
    let kwargs = auth_parameters();
    let call = JobStatusCall {
        batch_id: JOB_ARN,
        aws_region_name: Some(REGION),
        kwargs: &kwargs,
        now: now(),
    };
    let mut callback = |_batch: &str, _event: CallEvent<'_>| Err(PrePostError::RawRequestCaptured);
    let logger: &mut CallLogger<'_> = &mut callback;
    let error = runtime()
        .block_on(retrieve_model_job_status(
            call,
            &source,
            &factory,
            Some(logger),
        ))
        .expect_err("a logging failure propagates");
    assert!(matches!(error, BatchCallError::Logging(_)));
    assert_eq!(factory.requests().len(), 0);
    assert_eq!(factory.configs().len(), 1);
}
