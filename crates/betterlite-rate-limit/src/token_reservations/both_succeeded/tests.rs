#![cfg(test)]

use std::collections::BTreeSet;

use crate::limiter_records::{
    PROJECT_ITPM_DESCRIPTOR_KEY, PROJECT_OTPM_DESCRIPTOR_KEY, RateLimitStatus, RateLimitType,
    ReservationBackend,
};
use crate::token_reservations::tests::support::{
    ReservationFixture, START, count, identity, project_io_descriptors, project_io_reads,
    project_io_writes, text, tokens_key, window_key,
};

const VALUE: &str = "p1:gpt-4";

fn both_bucket_statuses() -> Vec<RateLimitStatus> {
    vec![
        RateLimitStatus {
            code: String::from("OK"),
            current_limit: 500,
            limit_remaining: 280,
            rate_limit_type: RateLimitType::Tokens,
            descriptor_key: String::from(PROJECT_ITPM_DESCRIPTOR_KEY),
            descriptor_value: Some(String::from(VALUE)),
        },
        RateLimitStatus {
            code: String::from("OK"),
            current_limit: 900,
            limit_remaining: 630,
            rate_limit_type: RateLimitType::Tokens,
            descriptor_key: String::from(PROJECT_OTPM_DESCRIPTOR_KEY),
            descriptor_value: Some(String::from(VALUE)),
        },
    ]
}

#[tokio::test]
async fn charges_both_buckets_and_merges_their_statuses_and_windows() {
    let fixture = ReservationFixture::seeded(&[
        (&window_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE), text(START)),
        (&tokens_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE), count(100)),
        (&window_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE), text(START)),
        (&tokens_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE), count(200)),
    ]);

    let reservation = fixture
        .reservations()
        .reserve_io_tokens(&project_io_descriptors(500, 900), 120, 70)
        .await
        .unwrap();

    assert_eq!(reservation.response.overall_code, "OK");
    assert_eq!(reservation.response.statuses, both_bucket_statuses());
    assert_eq!(
        reservation.response.reservation_windows,
        Some(BTreeSet::from([
            identity(
                &tokens_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE),
                &START.to_string(),
                ReservationBackend::Local
            ),
            identity(
                &tokens_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE),
                &START.to_string(),
                ReservationBackend::Local
            ),
        ]))
    );
    assert_eq!(reservation.itpm_reserved, 120);
    assert_eq!(reservation.otpm_reserved, 70);
    assert_eq!(fixture.effects.writes(), project_io_writes(220, 270));
}

#[tokio::test]
async fn reserves_each_bucket_against_its_own_estimate() {
    let fixture = ReservationFixture::seeded(&[
        (&window_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE), text(START)),
        (&tokens_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE), count(0)),
        (&window_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE), text(START)),
        (&tokens_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE), count(0)),
    ]);

    let reservation = fixture
        .reservations()
        .reserve_io_tokens(&project_io_descriptors(500, 900), 120, 70)
        .await
        .unwrap();

    assert_eq!(reservation.itpm_reserved, 120);
    assert_eq!(reservation.otpm_reserved, 70);
    assert_eq!(fixture.effects.reads(), project_io_reads());
    assert_eq!(fixture.effects.writes(), project_io_writes(120, 70));
}
