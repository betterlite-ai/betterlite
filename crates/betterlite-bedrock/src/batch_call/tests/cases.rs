use rstest::rstest;

mod cancel_idempotent_errors;
mod cancel_model_job;
mod completed_output_prediction;
mod handler_counts_and_timestamps;
mod logging_and_sdk_failures;
mod retrieve_async_status;
mod retrieve_model_job_status;

#[rstest]
#[case::cancel_model_job(cancel_model_job::cancel_model_job)]
#[case::retrieve_async_status(retrieve_async_status::retrieve_async_status)]
#[case::retrieve_model_job_status(retrieve_model_job_status::retrieve_model_job_status)]
#[case::cancel_idempotent_errors(cancel_idempotent_errors::cancel_idempotent_errors)]
#[case::completed_output_prediction(completed_output_prediction::completed_output_prediction)]
#[case::handler_counts_and_timestamps(handler_counts_and_timestamps::handler_counts_and_timestamps)]
#[case::logging_and_sdk_failures(logging_and_sdk_failures::logging_and_sdk_failures)]
fn batch_call_cases(#[case] case: fn()) {
    case();
}
