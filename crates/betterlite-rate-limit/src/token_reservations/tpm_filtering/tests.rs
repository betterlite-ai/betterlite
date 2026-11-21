#![cfg(test)]

use rstest::rstest;

use crate::limiter_records::{
    PROJECT_ITPM_DESCRIPTOR_KEY, PROJECT_OTPM_DESCRIPTOR_KEY, RateLimitDescriptor,
    RateLimitDescriptorRateLimit, RateLimitStatus, RateLimitType, ReservationBackend,
    ReservationWindowIdentity,
};
use crate::token_reservations::tests::support::{
    ReservationFixture, START, WriteRecord, count, counter_write, descriptor, text, tokens_key,
    window_key,
};
use crate::token_reservations::tpm_filtering::tpm_descriptors;

fn tpm_status(key: &str, value: &str, limit: i64, remaining: i64) -> RateLimitStatus {
    RateLimitStatus {
        code: "OK".to_owned(),
        current_limit: limit,
        limit_remaining: remaining,
        rate_limit_type: RateLimitType::Tokens,
        descriptor_key: key.to_owned(),
        descriptor_value: Some(value.to_owned()),
    }
}

fn window(key: &str, value: &str, start: &str) -> ReservationWindowIdentity {
    ReservationWindowIdentity {
        counter_key: tokens_key(key, value),
        window_start: start.to_owned(),
        backend: ReservationBackend::Local,
    }
}

fn charging_descriptors() -> Vec<RateLimitDescriptor> {
    vec![
        descriptor("model_per_key", "key1:gpt-4", Some(500), Some(60)),
        descriptor(
            PROJECT_ITPM_DESCRIPTOR_KEY,
            "proj:gpt-4",
            Some(1000),
            Some(60),
        ),
        descriptor(
            PROJECT_OTPM_DESCRIPTOR_KEY,
            "proj:gpt-4",
            Some(1000),
            Some(60),
        ),
        descriptor("model_per_team", "team1:gpt-4", Some(800), Some(60)),
    ]
}

fn charging_statuses() -> Vec<RateLimitStatus> {
    vec![
        tpm_status("model_per_key", "key1:gpt-4", 500, 130),
        tpm_status("model_per_team", "team1:gpt-4", 800, 580),
    ]
}

fn charging_writes() -> Vec<WriteRecord> {
    vec![
        counter_write(&tokens_key("model_per_key", "key1:gpt-4"), count(370), 60),
        counter_write(&tokens_key("model_per_team", "team1:gpt-4"), count(220), 60),
    ]
}

#[test]
fn drops_project_untokened_and_limitless_descriptors() {
    let descriptors = vec![
        descriptor("model_per_key", "key1:gpt-4", Some(500), Some(60)),
        descriptor(
            PROJECT_ITPM_DESCRIPTOR_KEY,
            "proj:gpt-4",
            Some(50),
            Some(60),
        ),
        descriptor(
            PROJECT_OTPM_DESCRIPTOR_KEY,
            "proj:gpt-4",
            Some(60),
            Some(60),
        ),
        descriptor("model_per_key", "key2:gpt-4", None, Some(60)),
        RateLimitDescriptor {
            key: String::from("model_per_team"),
            value: String::from("team:gpt-4"),
            rate_limit: None,
        },
    ];

    let filtered = tpm_descriptors(&descriptors);

    assert_eq!(
        filtered,
        vec![descriptor(
            "model_per_key",
            "key1:gpt-4",
            Some(500),
            Some(60)
        )]
    );
}

#[test]
fn keeps_only_the_tokens_limit_and_the_window_size() {
    let descriptors = vec![RateLimitDescriptor {
        key: String::from("model_per_key"),
        value: String::from("key1:gpt-4"),
        rate_limit: Some(RateLimitDescriptorRateLimit {
            requests_per_unit: Some(10),
            tokens_per_unit: Some(500),
            max_parallel_requests: Some(4),
            window_size: None,
        }),
    }];

    let filtered = tpm_descriptors(&descriptors);

    assert_eq!(filtered.len(), 1);
    let limits = filtered.first().and_then(|entry| entry.rate_limit.as_ref());
    assert_eq!(
        limits.cloned(),
        Some(RateLimitDescriptorRateLimit {
            requests_per_unit: None,
            tokens_per_unit: Some(500),
            max_parallel_requests: None,
            window_size: None,
        })
    );
}

#[rstest]
#[case::no_descriptors(Vec::new())]
#[case::only_project_descriptors(vec![
    descriptor(PROJECT_ITPM_DESCRIPTOR_KEY, "proj:gpt-4", Some(50), Some(60)),
    descriptor(PROJECT_OTPM_DESCRIPTOR_KEY, "proj:gpt-4", Some(60), Some(60)),
])]
#[case::only_untokened_descriptors(vec![descriptor("model_per_key", "key1:gpt-4", None, Some(60))])]
#[tokio::test]
async fn answers_ok_without_touching_the_cache_when_nothing_survives(
    #[case] descriptors: Vec<RateLimitDescriptor>,
) {
    let fixture = ReservationFixture::seeded(&[]);

    let response = fixture
        .reservations()
        .reserve_tpm_tokens(&descriptors, 100)
        .await
        .unwrap();

    assert_eq!(response.overall_code, "OK");
    assert!(response.statuses.is_empty());
    assert_eq!(response.reservation_windows, None);
    assert!(fixture.effects.reads().is_empty());
    assert!(fixture.effects.writes().is_empty());
}

#[tokio::test]
async fn charges_each_tpm_descriptor_and_skips_the_project_ones() {
    let fixture = ReservationFixture::seeded(&[
        (&tokens_key("model_per_key", "key1:gpt-4"), count(250)),
        (&window_key("model_per_key", "key1:gpt-4"), text(START)),
        (&tokens_key("model_per_team", "team1:gpt-4"), count(100)),
        (&window_key("model_per_team", "team1:gpt-4"), text(START)),
    ]);
    let descriptors = charging_descriptors();

    let response = fixture
        .reservations()
        .reserve_tpm_tokens(&descriptors, 120)
        .await
        .unwrap();

    assert_eq!(response.overall_code, "OK");
    assert_eq!(response.statuses, charging_statuses());
    assert_eq!(fixture.effects.writes(), charging_writes());
    assert_eq!(
        response.reservation_windows,
        Some(
            [
                window("model_per_key", "key1:gpt-4", &START.to_string()),
                window("model_per_team", "team1:gpt-4", &START.to_string()),
            ]
            .into_iter()
            .collect()
        )
    );
    assert!(
        !fixture
            .effects
            .reads()
            .iter()
            .any(|read| read.key.contains("proj:gpt-4"))
    );
}
