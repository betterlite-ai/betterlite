#![cfg(test)]

use crate::limiter_records::{
    PROJECT_ITPM_DESCRIPTOR_KEY, PROJECT_OTPM_DESCRIPTOR_KEY, RateLimitStatus, RateLimitType,
};
use crate::token_reservations::tests::support::{
    ReservationFixture, START, WriteRecord, count, counter_write, local_write,
    project_io_descriptors, project_io_reads, project_itpm_reads, text, tokens_key, window_key,
};

const VALUE: &str = "p1:gpt-4";

fn rollback_statuses() -> Vec<RateLimitStatus> {
    vec![RateLimitStatus {
        code: String::from("OVER_LIMIT"),
        current_limit: 50,
        limit_remaining: 10,
        rate_limit_type: RateLimitType::Tokens,
        descriptor_key: String::from(PROJECT_OTPM_DESCRIPTOR_KEY),
        descriptor_value: Some(String::from(VALUE)),
    }]
}

fn rollback_writes() -> Vec<WriteRecord> {
    vec![
        counter_write(
            &tokens_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE),
            count(400),
            60,
        ),
        local_write(
            &tokens_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE),
            300.0_f64,
            60,
        ),
    ]
}

#[tokio::test]
async fn refunds_the_itpm_reservation_when_the_otpm_bucket_is_over_limit() {
    let fixture = ReservationFixture::seeded(&[
        (&window_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE), text(START)),
        (&tokens_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE), count(300)),
        (&window_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE), text(START)),
        (&tokens_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE), count(40)),
    ]);

    let reservation = fixture
        .reservations()
        .reserve_io_tokens(&project_io_descriptors(500, 50), 100, 120)
        .await
        .unwrap();

    assert_eq!(reservation.response.overall_code, "OVER_LIMIT");
    assert_eq!(reservation.response.statuses, rollback_statuses());
    assert_eq!(reservation.itpm_reserved, 0);
    assert_eq!(reservation.otpm_reserved, 0);
    let mut expected_reads = project_io_reads();
    expected_reads.extend(project_itpm_reads());
    assert_eq!(fixture.effects.reads(), expected_reads);
    assert_eq!(fixture.effects.writes(), rollback_writes());
}

#[tokio::test]
async fn keeps_the_itpm_charge_when_the_input_estimate_is_zero() {
    let fixture = ReservationFixture::seeded(&[
        (&window_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE), text(START)),
        (&tokens_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE), count(0)),
        (&window_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE), text(START)),
        (&tokens_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE), count(40)),
    ]);

    let reservation = fixture
        .reservations()
        .reserve_io_tokens(&project_io_descriptors(500, 50), 0, 120)
        .await
        .unwrap();

    assert_eq!(reservation.response.overall_code, "OVER_LIMIT");
    assert_eq!(reservation.itpm_reserved, 0);
    assert_eq!(reservation.otpm_reserved, 0);
    assert_eq!(
        fixture.effects.writes(),
        vec![counter_write(
            &tokens_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE),
            count(0),
            60
        )]
    );
}
