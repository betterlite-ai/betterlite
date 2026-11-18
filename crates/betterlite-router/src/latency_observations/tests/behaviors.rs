use super::support::{
    DEPLOYMENT, Failure, GROUP, OTHER_GROUP, Success, TestClock, deployment, latency, minutes,
    record_elapsed, record_failure, record_success, samples, service, success_plan, ttft, usage,
};
use crate::cooldown_decisions::failure::FailureClass;
use crate::latency_observations::event::{Channel, RequestChannels};
use crate::latency_observations::samples::MinuteUsage;
use crate::latency_observations::settings::Settings;

pub(crate) fn run(name: &str) {
    match name {
        "a_success_records_the_response_time" => a_success_records_the_response_time(),
        "a_success_normalizes_the_response_time_by_completion_tokens" => {
            a_success_normalizes_the_response_time();
        }
        "a_zero_completion_token_count_keeps_the_response_time" => {
            a_zero_completion_count_keeps_the_response_time();
        }
        "a_streaming_success_records_the_first_token_time" => a_streaming_success_records_ttft(),
        "a_streaming_success_without_a_completion_start_uses_the_end_time" => {
            a_streaming_success_without_a_completion_start();
        }
        "a_non_streaming_success_records_no_first_token_time" => {
            a_non_streaming_success_has_no_ttft()
        }
        "the_minute_bucket_sums_tokens_and_counts_requests" => the_minute_bucket_sums_tokens(),
        "a_timeout_failure_appends_the_penalty" => a_timeout_failure_appends_the_penalty(),
        "a_non_timeout_failure_records_nothing" => a_non_timeout_failure_records_nothing(),
        "a_batch_retrieve_callback_records_nothing" => a_batch_retrieve_callback_records_nothing(),
        "litellm_metadata_is_read_before_metadata" => litellm_metadata_is_read_before_metadata(),
        "a_missing_model_group_or_deployment_records_nothing" => {
            a_missing_group_or_deployment_records_nothing();
        }
        "the_newest_minute_starts_its_own_bucket" => the_newest_minute_starts_its_own_bucket(),
        "an_unknown_group_has_no_samples" => an_unknown_group_has_no_samples(),
        other => panic!("unknown behavior case {other}"),
    }
}

fn a_success_records_the_response_time() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    record_elapsed(&observations, &[2_000]);
    assert_eq!(latency(&observations), vec![2.0_f64]);
    assert_eq!(
        minutes(&observations),
        vec![(
            "1970-01-01-00-00".to_owned(),
            MinuteUsage { tpm: 0, rpm: 1 }
        )]
    );
}

fn a_success_normalizes_the_response_time() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    let mut plan = Success::default();
    plan.elapsed_millis = 2_000;
    plan.usage = usage(4, 10);
    record_success(&observations, &plan);
    assert_eq!(latency(&observations), vec![0.5_f64]);
    assert_eq!(
        minutes(&observations),
        vec![(
            "1970-01-01-00-00".to_owned(),
            MinuteUsage { tpm: 10, rpm: 1 }
        )]
    );
}

fn a_zero_completion_count_keeps_the_response_time() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    record_success(&observations, &success_plan(2_500, usage(0, 7)));
    let recorded = minutes(&observations);
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].1.tpm, 7);
    assert_eq!(recorded[0].1.rpm, 1);
    assert_eq!(latency(&observations), vec![2.5_f64]);
}

fn a_streaming_success_records_ttft() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    let mut plan = Success::default();
    plan.stream = true;
    plan.elapsed_millis = 2_000;
    plan.completion_start_millis = Some(250);
    plan.usage = usage(4, 10);
    record_success(&observations, &plan);
    assert_eq!(latency(&observations), vec![0.5_f64]);
    assert_eq!(ttft(&observations), vec![0.25_f64]);
}

fn a_streaming_success_without_a_completion_start() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    let mut plan = Success::default();
    plan.stream = true;
    plan.elapsed_millis = 1_500;
    record_success(&observations, &plan);
    assert_eq!(latency(&observations), vec![1.5_f64]);
    assert_eq!(ttft(&observations), vec![1.5_f64]);
}

fn a_non_streaming_success_has_no_ttft() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    let mut plan = Success::default();
    plan.completion_start_millis = Some(250);
    record_success(&observations, &plan);
    assert_eq!(latency(&observations), vec![1.0_f64]);
    assert!(ttft(&observations).is_empty());
}

fn the_minute_bucket_sums_tokens() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    let mut first = Success::default();
    first.elapsed_millis = 2_000;
    first.usage = usage(4, 10);
    record_success(&observations, &first);
    clock.set(5_000);
    let mut second = Success::default();
    second.elapsed_millis = 1_000;
    second.usage = usage(2, 30);
    record_success(&observations, &second);
    assert_eq!(latency(&observations), vec![0.5_f64, 0.5_f64]);
    assert_eq!(
        minutes(&observations),
        vec![(
            "1970-01-01-00-00".to_owned(),
            MinuteUsage { tpm: 40, rpm: 2 }
        )]
    );
}

fn a_timeout_failure_appends_the_penalty() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    record_failure(&observations, &Failure::default());
    assert_eq!(latency(&observations), vec![1000.0_f64]);
    assert!(minutes(&observations).is_empty());
    assert!(ttft(&observations).is_empty());
}

fn a_non_timeout_failure_records_nothing() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    let mut plan = Failure::default();
    plan.failure = FailureClass::Api;
    record_failure(&observations, &plan);
    assert!(samples(&observations, GROUP).deployments.is_empty());
}

fn a_batch_retrieve_callback_records_nothing() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    let mut success = Success::default();
    success.call_type = Some("retrieve_batch");
    record_success(&observations, &success);
    let mut failure = Failure::default();
    failure.call_type = Some("aretrieve_batch");
    record_failure(&observations, &failure);
    assert!(samples(&observations, GROUP).deployments.is_empty());
}

fn litellm_metadata_is_read_before_metadata() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    let mut plan = Success::default();
    plan.channels = RequestChannels {
        litellm_metadata: Channel::Present(Some(OTHER_GROUP)),
        metadata: Channel::Present(Some(GROUP)),
    };
    record_success(&observations, &plan);
    assert!(samples(&observations, GROUP).deployments.is_empty());
    assert_eq!(
        deployment(&samples(&observations, OTHER_GROUP), DEPLOYMENT).latency,
        vec![1.0_f64]
    );
}

fn a_missing_group_or_deployment_records_nothing() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    let mut absent = Success::default();
    absent.channels = RequestChannels {
        litellm_metadata: Channel::Absent,
        metadata: Channel::Absent,
    };
    record_success(&observations, &absent);
    let mut empty_channel = Success::default();
    empty_channel.channels = RequestChannels {
        litellm_metadata: Channel::Present(None),
        metadata: Channel::Present(Some(GROUP)),
    };
    record_success(&observations, &empty_channel);
    let mut no_deployment = Success::default();
    no_deployment.deployment_id = None;
    record_success(&observations, &no_deployment);
    assert!(samples(&observations, GROUP).deployments.is_empty());
}

fn the_newest_minute_starts_its_own_bucket() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    record_success(&observations, &Success::default());
    clock.set(60_000);
    record_success(&observations, &Success::default());
    assert_eq!(
        minutes(&observations),
        vec![
            (
                "1970-01-01-00-00".to_owned(),
                MinuteUsage { tpm: 0, rpm: 1 }
            ),
            (
                "1970-01-01-00-01".to_owned(),
                MinuteUsage { tpm: 0, rpm: 1 }
            ),
        ]
    );
}

fn an_unknown_group_has_no_samples() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    assert!(samples(&observations, OTHER_GROUP).deployments.is_empty());
}
