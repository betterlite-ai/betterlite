#![cfg(test)]

use crate::token_reservations::tests::support::{ReservationFixture, auth};

#[tokio::test]
async fn returns_before_any_charge_without_an_authenticated_caller() {
    let fixture = ReservationFixture::seeded(&[]);

    fixture
        .reservations()
        .enforce_frame_project_io_quota(None, Some("gpt-4"), 120, 70)
        .await
        .unwrap();

    assert!(fixture.effects.reads().is_empty());
    assert!(fixture.effects.writes().is_empty());
    assert!(fixture.effects.increments().is_empty());
    assert!(fixture.effects.pipelines().is_empty());
}

#[tokio::test]
async fn returns_before_any_charge_without_a_requested_model() {
    let fixture = ReservationFixture::seeded(&[]);
    let auth =
        auth(r#"{"project_id": "p1", "project_metadata": {"model_itpm_limit": {"gpt-4": 500}}}"#);

    fixture
        .reservations()
        .enforce_frame_project_io_quota(Some(&auth), None, 120, 70)
        .await
        .unwrap();

    assert!(fixture.effects.reads().is_empty());
    assert!(fixture.effects.writes().is_empty());
}
