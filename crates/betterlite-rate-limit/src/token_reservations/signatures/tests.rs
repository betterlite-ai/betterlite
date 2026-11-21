#![cfg(test)]

use std::collections::BTreeSet;

use crate::limiter_records::{RateLimitDescriptor, RateLimitResponse, ReservationWindowIdentity};
use crate::token_reservations::tests::support::{
    ReservationFixture, START, count, counter_write, descriptor, local_write, scope, text,
    tokens_key, window_key,
};
use crate::token_reservations::{IoReservation, TokenReservationError, TokenReservations};

const KEY: &str = "model_per_key";
const VALUE: &str = "key1:gpt-4";

#[tokio::test]
async fn the_entry_points_take_the_litellm_argument_shapes() {
    let fixture = ReservationFixture::seeded(&[]);
    let reservations: TokenReservations<'_> = fixture.reservations();
    let descriptors: &[RateLimitDescriptor] = &[descriptor(KEY, VALUE, Some(500), Some(60))];

    let reserved: Result<RateLimitResponse, TokenReservationError> =
        reservations.reserve_tpm_tokens(descriptors, 10).await;
    assert_eq!(reserved.unwrap().overall_code, "OK");

    let reservation: IoReservation = reservations
        .reserve_io_tokens(descriptors, 120, 70)
        .await
        .unwrap();
    let (response, itpm_reserved, otpm_reserved): (RateLimitResponse, i64, i64) = (
        reservation.response,
        reservation.itpm_reserved,
        reservation.otpm_reserved,
    );
    assert_eq!(response.overall_code, "OK");
    assert_eq!(itpm_reserved, 0);
    assert_eq!(otpm_reserved, 0);

    let scopes: Vec<(String, String)> = vec![scope(KEY, VALUE)];
    let windows: BTreeSet<ReservationWindowIdentity> = BTreeSet::new();
    let refunded: Result<(), TokenReservationError> = reservations
        .refund_reserved_tokens(&scopes, 0, &windows)
        .await;
    refunded.unwrap();

    let frame: Result<(), TokenReservationError> = reservations
        .enforce_frame_project_io_quota(None, Some("gpt-4"), 120, 70)
        .await;
    frame.unwrap();
}

#[tokio::test]
async fn the_refund_takes_the_windows_the_reservation_reported() {
    let fixture = ReservationFixture::seeded(&[
        (&window_key(KEY, VALUE), text(START)),
        (&tokens_key(KEY, VALUE), count(0)),
    ]);
    let reservations = fixture.reservations();
    let descriptors = [descriptor(KEY, VALUE, Some(500), Some(60))];

    let response = reservations
        .reserve_tpm_tokens(&descriptors, 10)
        .await
        .unwrap();
    let windows: BTreeSet<ReservationWindowIdentity> = response
        .reservation_windows
        .expect("the charge reports its window");

    reservations
        .refund_reserved_tokens(&[scope(KEY, VALUE)], 10, &windows)
        .await
        .unwrap();

    assert_eq!(
        fixture.effects.writes(),
        vec![
            counter_write(&tokens_key(KEY, VALUE), count(10), 60),
            local_write(&tokens_key(KEY, VALUE), 0.0_f64, 60),
        ]
    );
}
