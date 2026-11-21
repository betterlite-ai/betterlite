#![cfg(test)]

use rstest::rstest;

use crate::limiter_records::{
    PROJECT_ITPM_DESCRIPTOR_KEY, PROJECT_OTPM_DESCRIPTOR_KEY, RateLimitStatus, RateLimitType,
};
use crate::token_reservations::tests::support::{
    ReservationFixture, START, count, itpm_descriptors, project_io_descriptors, project_itpm_reads,
    text, tokens_key, window_key,
};

const VALUE: &str = "p1:gpt-4";

fn itpm_over_limit_statuses() -> Vec<RateLimitStatus> {
    vec![RateLimitStatus {
        code: String::from("OVER_LIMIT"),
        current_limit: 50,
        limit_remaining: 10,
        rate_limit_type: RateLimitType::Tokens,
        descriptor_key: String::from(PROJECT_ITPM_DESCRIPTOR_KEY),
        descriptor_value: Some(String::from(VALUE)),
    }]
}

#[rstest]
#[case::just_inside(40, 140, "OK", 100)]
#[case::just_outside(40, 139, "OVER_LIMIT", 0)]
#[tokio::test]
async fn the_input_estimate_decides_whether_the_itpm_bucket_fits(
    #[case] current: i64,
    #[case] limit: i64,
    #[case] code: &str,
    #[case] itpm_reserved: i64,
) {
    let fixture = ReservationFixture::seeded(&[
        (&window_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE), text(START)),
        (
            &tokens_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE),
            count(current),
        ),
    ]);

    let reservation = fixture
        .reservations()
        .reserve_io_tokens(&itpm_descriptors(limit), 100, 50)
        .await
        .unwrap();

    assert_eq!(reservation.response.overall_code, code);
    assert_eq!(reservation.itpm_reserved, itpm_reserved);
    assert_eq!(reservation.otpm_reserved, 0);
}

#[tokio::test]
async fn stops_before_charging_otpm_when_itpm_is_over_limit() {
    let fixture = ReservationFixture::seeded(&[
        (&window_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE), text(START)),
        (&tokens_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE), count(40)),
        (&window_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE), text(START)),
        (&tokens_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE), count(10)),
    ]);

    let reservation = fixture
        .reservations()
        .reserve_io_tokens(&project_io_descriptors(50, 900), 100, 50)
        .await
        .unwrap();

    assert_eq!(reservation.response.overall_code, "OVER_LIMIT");
    assert_eq!(reservation.response.statuses, itpm_over_limit_statuses());
    assert_eq!(reservation.itpm_reserved, 0);
    assert_eq!(reservation.otpm_reserved, 0);
    assert_eq!(fixture.effects.reads(), project_itpm_reads());
    assert!(fixture.effects.writes().is_empty());
}
