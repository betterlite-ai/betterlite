#![cfg(test)]

use crate::limiter_records::{PROJECT_ITPM_DESCRIPTOR_KEY, PROJECT_OTPM_DESCRIPTOR_KEY};
use crate::token_reservations::tests::support::{
    ReservationFixture, START, count, descriptor, text, tokens_key, window_key,
};

const VALUE: &str = "p1:gpt-4";

#[tokio::test]
async fn reports_the_input_estimate_for_a_reserved_itpm_bucket() {
    let fixture = ReservationFixture::seeded(&[
        (&window_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE), text(START)),
        (&tokens_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE), count(0)),
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
    assert_eq!(reservation.itpm_reserved, 120);
    assert_eq!(reservation.otpm_reserved, 0);
}

#[tokio::test]
async fn reports_zero_when_only_otpm_is_configured() {
    let fixture = ReservationFixture::seeded(&[
        (&window_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE), text(START)),
        (&tokens_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE), count(0)),
    ]);

    let reservation = fixture
        .reservations()
        .reserve_io_tokens(
            &[descriptor(
                PROJECT_OTPM_DESCRIPTOR_KEY,
                VALUE,
                Some(900),
                Some(60),
            )],
            120,
            70,
        )
        .await
        .unwrap();

    assert_eq!(reservation.response.overall_code, "OK");
    assert_eq!(reservation.itpm_reserved, 0);
    assert_eq!(reservation.otpm_reserved, 70);
}
