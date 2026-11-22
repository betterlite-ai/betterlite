#![cfg(test)]

use crate::token_reservations::tests::support::{ReservationFixture, auth};

#[tokio::test]
async fn returns_before_any_charge_without_project_io_limits() {
    let fixture = ReservationFixture::seeded(&[]);
    let auth = auth(r#"{"project_id": "p1"}"#);

    fixture
        .reservations()
        .enforce_frame_project_io_quota(Some(&auth), Some("gpt-4"), 120, 70)
        .await
        .unwrap();

    assert!(fixture.effects.reads().is_empty());
    assert!(fixture.effects.writes().is_empty());
}

#[tokio::test]
async fn returns_before_any_charge_without_a_project() {
    let fixture = ReservationFixture::seeded(&[]);
    let auth = auth(
        r#"{"project_metadata": {"model_itpm_limit": {"gpt-4": 500}, "model_otpm_limit": {"gpt-4": 900}}}"#,
    );

    fixture
        .reservations()
        .enforce_frame_project_io_quota(Some(&auth), Some("gpt-4"), 120, 70)
        .await
        .unwrap();

    assert!(fixture.effects.reads().is_empty());
    assert!(fixture.effects.writes().is_empty());
}
