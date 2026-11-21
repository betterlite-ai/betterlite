#![cfg(test)]

use std::collections::BTreeSet;

use crate::limiter_records::{
    PROJECT_ITPM_DESCRIPTOR_KEY, RateLimitStatus, RateLimitType, ReservationBackend,
};
use crate::token_reservations::tests::support::{
    ReservationFixture, START, count, counter_write, descriptor, identity, read, text, tokens_key,
    window_key,
};

const VALUE: &str = "p1:gpt-4";

#[tokio::test]
async fn answers_the_itpm_response_without_reserving_the_output_estimate() {
    let fixture = ReservationFixture::seeded(&[
        (&window_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE), text(START)),
        (&tokens_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE), count(40)),
    ]);

    let reservation = fixture
        .reservations()
        .reserve_io_tokens(
            &[descriptor(
                PROJECT_ITPM_DESCRIPTOR_KEY,
                VALUE,
                Some(500),
                Some(60),
            )],
            120,
            70,
        )
        .await
        .unwrap();

    assert_eq!(reservation.response.overall_code, "OK");
    assert_eq!(
        reservation.response.statuses,
        vec![RateLimitStatus {
            code: String::from("OK"),
            current_limit: 500,
            limit_remaining: 340,
            rate_limit_type: RateLimitType::Tokens,
            descriptor_key: String::from(PROJECT_ITPM_DESCRIPTOR_KEY),
            descriptor_value: Some(String::from(VALUE)),
        }]
    );
    assert_eq!(
        reservation.response.reservation_windows,
        Some(BTreeSet::from([identity(
            &tokens_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE),
            &START.to_string(),
            ReservationBackend::Local
        )]))
    );
    assert_eq!(reservation.itpm_reserved, 120);
    assert_eq!(reservation.otpm_reserved, 0);
    assert_eq!(
        fixture.effects.reads(),
        vec![
            read(&window_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE)),
            read(&tokens_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE)),
        ]
    );
    assert_eq!(
        fixture.effects.writes(),
        vec![counter_write(
            &tokens_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE),
            count(160),
            60
        )]
    );
}
