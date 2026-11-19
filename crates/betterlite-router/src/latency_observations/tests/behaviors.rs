use super::support::{
    DEFAULT_FAILURE, DEFAULT_SUCCESS, DEPLOYMENT, Failure, GROUP, OTHER_GROUP, Success, TestClock,
    deployment, latency, minutes, record_elapsed, record_failure, record_success, samples, service,
    success_plan, ttft, usage,
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
            a_non_streaming_success_has_no_ttft();
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
    let plan = success_plan(2_000, Some(usage(4, 10)));
    record_success(&observations, &plan);
    assert_eq!(latency(&observations), vec![0.5_f64]);
    let recorded = minutes(&observations);
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].1, MinuteUsage { tpm: 10, rpm: 1 });
}

fn a_zero_completion_count_keeps_the_response_time() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    record_success(&observations, &success_plan(2_500, Some(usage(0, 7))));
    let recorded = minutes(&observations);
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].1.tpm, 7);
    assert_eq!(recorded[0].1.rpm, 1);
    assert_eq!(latency(&observations), vec![2.5_f64]);
}

fn a_streaming_success_records_ttft() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    let first_token = 250;
    let plan = Success {
        stream: true,
        elapsed_millis: 2_000,
        completion_start_millis: Some(first_token),
        usage: Some(usage(4, 10)),
        ..DEFAULT_SUCCESS
    };
    record_success(&observations, &plan);
    assert_eq!(latency(&observations), vec![0.5_f64]);
    assert_eq!(ttft(&observations), vec![0.25_f64]);
}

fn a_streaming_success_without_a_completion_start() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    record_success(
        &observations,
        &Success {
            stream: true,
            elapsed_millis: 1_500,
            ..DEFAULT_SUCCESS
        },
    );
    let recorded = deployment(&samples(&observations, GROUP), DEPLOYMENT);
    assert_eq!(recorded.latency, vec![1.5_f64]);
    assert_eq!(recorded.time_to_first_token_seconds, vec![1.5_f64]);
}

fn a_non_streaming_success_has_no_ttft() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    record_success(
        &observations,
        &Success {
            completion_start_millis: Some(250),
            ..DEFAULT_SUCCESS
        },
    );
    assert_eq!(latency(&observations), vec![1.0_f64]);
    assert!(ttft(&observations).is_empty());
}

fn the_minute_bucket_sums_tokens() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    record_success(&observations, &success_plan(2_000, Some(usage(4, 10))));
    clock.set(5_000);
    record_success(&observations, &success_plan(1_000, Some(usage(2, 30))));
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
    record_failure(
        &observations,
        &Failure {
            class: FailureClass::Api,
            ..DEFAULT_FAILURE
        },
    );
    assert!(samples(&observations, GROUP).deployments.is_empty());
}

fn a_batch_retrieve_callback_records_nothing() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    record_success(
        &observations,
        &Success {
            call_type: Some("retrieve_batch"),
            ..DEFAULT_SUCCESS
        },
    );
    record_failure(
        &observations,
        &Failure {
            call_type: Some("aretrieve_batch"),
            ..DEFAULT_FAILURE
        },
    );
    assert!(samples(&observations, GROUP).deployments.is_empty());
}

fn litellm_metadata_is_read_before_metadata() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    record_success(
        &observations,
        &Success {
            channels: RequestChannels {
                litellm_metadata: Channel::Present(Some(OTHER_GROUP)),
                metadata: Channel::Present(Some(GROUP)),
            },
            ..DEFAULT_SUCCESS
        },
    );
    assert!(samples(&observations, GROUP).deployments.is_empty());
    assert_eq!(
        deployment(&samples(&observations, OTHER_GROUP), DEPLOYMENT).latency,
        vec![1.0_f64]
    );
}

fn a_missing_group_or_deployment_records_nothing() {
    let clock = TestClock::at(0);
    let observations = service(Settings::default(), &clock);
    record_success(
        &observations,
        &Success {
            channels: RequestChannels {
                litellm_metadata: Channel::Absent,
                metadata: Channel::Absent,
            },
            ..DEFAULT_SUCCESS
        },
    );
    record_success(
        &observations,
        &Success {
            channels: RequestChannels {
                litellm_metadata: Channel::Present(None),
                metadata: Channel::Present(Some(GROUP)),
            },
            ..DEFAULT_SUCCESS
        },
    );
    record_success(
        &observations,
        &Success {
            deployment_id: None,
            ..DEFAULT_SUCCESS
        },
    );
    record_failure(
        &observations,
        &Failure {
            deployment_id: None,
            ..DEFAULT_FAILURE
        },
    );
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
