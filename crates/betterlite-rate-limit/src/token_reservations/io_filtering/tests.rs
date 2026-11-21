#![cfg(test)]

use crate::limiter_records::{PROJECT_ITPM_DESCRIPTOR_KEY, PROJECT_OTPM_DESCRIPTOR_KEY};
use crate::token_reservations::io_filtering::split_io_descriptors;
use crate::token_reservations::tests::support::{ReservationFixture, descriptor};

#[test]
fn separates_the_io_descriptors_by_key_and_keeps_their_order() {
    let itpm_first = descriptor(PROJECT_ITPM_DESCRIPTOR_KEY, "p1:gpt-4", Some(100), Some(60));
    let itpm_second = descriptor(PROJECT_ITPM_DESCRIPTOR_KEY, "p2:gpt-4", Some(200), Some(60));
    let otpm = descriptor(PROJECT_OTPM_DESCRIPTOR_KEY, "p1:gpt-4", Some(300), Some(60));
    let other = descriptor("model_per_key", "key1:gpt-4", Some(400), Some(60));
    let descriptors = vec![itpm_first.clone(), other, otpm.clone(), itpm_second.clone()];

    let (itpm, otpm_split) = split_io_descriptors(&descriptors);

    assert_eq!(itpm, vec![itpm_first, itpm_second]);
    assert_eq!(otpm_split, vec![otpm]);
}

#[tokio::test]
async fn answers_an_empty_ok_reservation_when_no_io_descriptor_exists() {
    let fixture = ReservationFixture::seeded(&[]);
    let descriptors = vec![
        descriptor("model_per_key", "key1:gpt-4", Some(400), Some(60)),
        descriptor("model_per_team", "team1:gpt-4", Some(400), Some(60)),
    ];

    let reservation = fixture
        .reservations()
        .reserve_io_tokens(&descriptors, 120, 70)
        .await
        .unwrap();

    assert_eq!(reservation.response.overall_code, "OK");
    assert!(reservation.response.statuses.is_empty());
    assert_eq!(reservation.response.reservation_windows, None);
    assert_eq!(reservation.itpm_reserved, 0);
    assert_eq!(reservation.otpm_reserved, 0);
    assert!(fixture.effects.reads().is_empty());
    assert!(fixture.effects.writes().is_empty());
}
