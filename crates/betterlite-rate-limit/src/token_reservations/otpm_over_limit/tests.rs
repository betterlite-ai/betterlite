#![cfg(test)]

use rstest::rstest;

use crate::limiter_records::PROJECT_OTPM_DESCRIPTOR_KEY;
use crate::token_reservations::tests::support::{
    ReservationFixture, START, count, descriptor, read, text, tokens_key, window_key,
};

const VALUE: &str = "p1:gpt-4";

#[rstest]
#[case::just_inside(40, 140, "OK", 100)]
#[case::just_outside(40, 139, "OVER_LIMIT", 0)]
#[tokio::test]
async fn the_output_estimate_decides_whether_the_otpm_bucket_fits(
    #[case] current: i64,
    #[case] limit: i64,
    #[case] code: &str,
    #[case] otpm_reserved: i64,
) {
    let fixture = ReservationFixture::seeded(&[
        (&window_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE), text(START)),
        (
            &tokens_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE),
            count(current),
        ),
    ]);

    let reservation = fixture
        .reservations()
        .reserve_io_tokens(
            &[descriptor(
                PROJECT_OTPM_DESCRIPTOR_KEY,
                VALUE,
                Some(limit),
                Some(60),
            )],
            120,
            100,
        )
        .await
        .unwrap();

    assert_eq!(reservation.response.overall_code, code);
    assert_eq!(reservation.itpm_reserved, 0);
    assert_eq!(reservation.otpm_reserved, otpm_reserved);
}

#[tokio::test]
async fn refunds_nothing_when_itpm_reserved_nothing() {
    let fixture = ReservationFixture::seeded(&[
        (&window_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE), text(START)),
        (&tokens_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE), count(40)),
    ]);

    let reservation = fixture
        .reservations()
        .reserve_io_tokens(
            &[descriptor(
                PROJECT_OTPM_DESCRIPTOR_KEY,
                VALUE,
                Some(50),
                Some(60),
            )],
            120,
            100,
        )
        .await
        .unwrap();

    assert_eq!(reservation.response.overall_code, "OVER_LIMIT");
    assert_eq!(reservation.itpm_reserved, 0);
    assert_eq!(reservation.otpm_reserved, 0);
    assert_eq!(
        fixture.effects.reads(),
        vec![
            read(&window_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE)),
            read(&tokens_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE)),
        ]
    );
    assert!(fixture.effects.writes().is_empty());
}
