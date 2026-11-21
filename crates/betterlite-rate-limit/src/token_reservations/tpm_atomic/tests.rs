#![cfg(test)]

use crate::limiter_records::{
    RateLimitDescriptor, RateLimitDescriptorRateLimit, RateLimitStatus, RateLimitType,
    ReservationBackend, ReservationWindowIdentity, create_rate_limit_keys,
};
use crate::token_reservations::tests::support::{
    NOW, ReservationFixture, START, count, counter_write, descriptor, key_descriptors, read, text,
    tokens_key, window_key,
};

fn requests_key(key: &str, value: &str) -> String {
    create_rate_limit_keys(key, value, RateLimitType::Requests)
}

fn descriptor_with_requests() -> RateLimitDescriptor {
    RateLimitDescriptor {
        key: String::from("model_per_key"),
        value: String::from("key1:gpt-4"),
        rate_limit: Some(RateLimitDescriptorRateLimit {
            requests_per_unit: Some(5),
            tokens_per_unit: Some(500),
            max_parallel_requests: None,
            window_size: Some(60),
        }),
    }
}

fn token_status(code: &str, value: &str, limit: i64, remaining: i64) -> RateLimitStatus {
    RateLimitStatus {
        code: code.to_owned(),
        current_limit: limit,
        limit_remaining: remaining,
        rate_limit_type: RateLimitType::Tokens,
        descriptor_key: String::from("model_per_key"),
        descriptor_value: Some(value.to_owned()),
    }
}

#[tokio::test]
async fn never_charges_the_requests_counter_of_a_descriptor() {
    let fixture = ReservationFixture::seeded(&[
        (&window_key("model_per_key", "key1:gpt-4"), text(START)),
        (&requests_key("model_per_key", "key1:gpt-4"), count(3)),
        (&tokens_key("model_per_key", "key1:gpt-4"), count(250)),
    ]);
    let descriptors = vec![descriptor_with_requests()];

    let response = fixture
        .reservations()
        .reserve_tpm_tokens(&descriptors, 120)
        .await
        .unwrap();

    assert_eq!(
        response.statuses,
        vec![token_status("OK", "key1:gpt-4", 500, 130)]
    );
    assert_eq!(
        fixture.effects.writes(),
        vec![counter_write(
            &tokens_key("model_per_key", "key1:gpt-4"),
            count(370),
            60
        )]
    );
    assert!(
        !fixture
            .effects
            .reads()
            .iter()
            .any(|entry| entry.key.ends_with(":requests"))
    );
}

#[tokio::test]
async fn returns_the_settlement_over_limit_response() {
    let fixture = ReservationFixture::seeded(&[
        (&window_key("model_per_key", "key1:gpt-4"), text(START)),
        (&tokens_key("model_per_key", "key1:gpt-4"), count(450)),
    ]);

    let response = fixture
        .reservations()
        .reserve_tpm_tokens(
            &[descriptor(
                "model_per_key",
                "key1:gpt-4",
                Some(500),
                Some(60),
            )],
            100,
        )
        .await
        .unwrap();

    assert_eq!(response.overall_code, "OVER_LIMIT");
    assert_eq!(
        response.statuses,
        vec![token_status("OVER_LIMIT", "key1:gpt-4", 500, 50)]
    );
    assert_eq!(response.reservation_windows, None);
    assert!(fixture.effects.writes().is_empty());
    assert_eq!(
        fixture.effects.reads(),
        vec![
            read(&window_key("model_per_key", "key1:gpt-4")),
            read(&tokens_key("model_per_key", "key1:gpt-4")),
        ]
    );
}

#[tokio::test]
async fn charges_a_repeated_scope_from_one_snapshot_per_descriptor() {
    let fixture = ReservationFixture::seeded(&[
        (&window_key("model_per_key", "key1:gpt-4"), text(START)),
        (&tokens_key("model_per_key", "key1:gpt-4"), count(250)),
    ]);
    let repeated = descriptor("model_per_key", "key1:gpt-4", Some(500), Some(60));

    let response = fixture
        .reservations()
        .reserve_tpm_tokens(&[repeated.clone(), repeated], 120)
        .await
        .unwrap();

    assert_eq!(
        response.statuses,
        vec![
            token_status("OK", "key1:gpt-4", 500, 130),
            token_status("OK", "key1:gpt-4", 500, 130),
        ]
    );
    assert_eq!(
        fixture.effects.writes(),
        vec![
            counter_write(&tokens_key("model_per_key", "key1:gpt-4"), count(370), 60),
            counter_write(&tokens_key("model_per_key", "key1:gpt-4"), count(370), 60),
        ]
    );
}

#[tokio::test]
async fn starts_a_new_window_and_resets_the_sibling_counters() {
    let fixture = ReservationFixture::seeded(&[]);

    let response = fixture
        .reservations()
        .reserve_tpm_tokens(&key_descriptors(500, Some(60)), 100)
        .await
        .unwrap();

    assert_eq!(response.overall_code, "OK");
    assert_eq!(
        response.reservation_windows,
        Some(
            [ReservationWindowIdentity {
                counter_key: tokens_key("model_per_key", "key1:gpt-4"),
                window_start: NOW.to_string(),
                backend: ReservationBackend::Local,
            }]
            .into_iter()
            .collect()
        )
    );
    assert_eq!(
        fixture.effects.reads(),
        vec![read(&window_key("model_per_key", "key1:gpt-4"))]
    );
    assert_eq!(
        fixture.effects.writes(),
        vec![
            counter_write(&requests_key("model_per_key", "key1:gpt-4"), count(0), 60),
            counter_write(&tokens_key("model_per_key", "key1:gpt-4"), count(0), 60),
            counter_write(&window_key("model_per_key", "key1:gpt-4"), text(NOW), 60),
            counter_write(&tokens_key("model_per_key", "key1:gpt-4"), count(100), 60),
        ]
    );
}

#[tokio::test]
async fn keeps_the_descriptor_window_size_over_the_default() {
    let fixture = ReservationFixture::seeded(&[
        (&window_key("model_per_key", "key1:gpt-4"), text(START)),
        (&tokens_key("model_per_key", "key1:gpt-4"), count(0)),
        (&window_key("model_per_team", "team1:gpt-4"), text(START)),
        (&tokens_key("model_per_team", "team1:gpt-4"), count(0)),
    ]);

    fixture
        .reservations()
        .reserve_tpm_tokens(
            &[
                descriptor("model_per_key", "key1:gpt-4", Some(500), Some(120)),
                descriptor("model_per_team", "team1:gpt-4", Some(500), None),
            ],
            100,
        )
        .await
        .unwrap();

    assert_eq!(
        fixture.effects.writes(),
        vec![
            counter_write(&tokens_key("model_per_key", "key1:gpt-4"), count(100), 120),
            counter_write(&tokens_key("model_per_team", "team1:gpt-4"), count(100), 60),
        ]
    );
}
