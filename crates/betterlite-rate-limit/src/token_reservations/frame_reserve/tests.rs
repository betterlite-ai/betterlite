#![cfg(test)]

use crate::limiter_records::{PROJECT_ITPM_DESCRIPTOR_KEY, PROJECT_OTPM_DESCRIPTOR_KEY};
use crate::token_reservations::tests::support::{
    ReservationFixture, START, auth, count, counter_write, text, tokens_key, window_key,
};

const VALUE: &str = "p1:gpt-4";

#[tokio::test]
async fn charges_the_frame_estimates_against_the_project_buckets() {
    let fixture = ReservationFixture::seeded(&[
        (&window_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE), text(START)),
        (&tokens_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE), count(100)),
        (&window_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE), text(START)),
        (&tokens_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE), count(0)),
    ]);
    let auth = auth(
        r#"{"project_id": "p1", "project_metadata": {"model_itpm_limit": {"gpt-4": 500}, "model_otpm_limit": {"gpt-4": 900}}}"#,
    );

    fixture
        .reservations()
        .enforce_frame_project_io_quota(Some(&auth), Some("gpt-4"), 120, 70)
        .await
        .unwrap();

    assert_eq!(
        fixture.effects.writes(),
        vec![
            counter_write(
                &tokens_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE),
                count(220),
                60
            ),
            counter_write(
                &tokens_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE),
                count(70),
                60
            ),
        ]
    );
}

#[tokio::test]
async fn charges_only_the_input_bucket_when_only_itpm_is_configured() {
    let fixture = ReservationFixture::seeded(&[
        (&window_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE), text(START)),
        (&tokens_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE), count(0)),
    ]);
    let auth =
        auth(r#"{"project_id": "p1", "project_metadata": {"model_itpm_limit": {"gpt-4": 500}}}"#);

    fixture
        .reservations()
        .enforce_frame_project_io_quota(Some(&auth), Some("gpt-4"), 120, 70)
        .await
        .unwrap();

    assert_eq!(
        fixture.effects.writes(),
        vec![counter_write(
            &tokens_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE),
            count(120),
            60
        )]
    );
}
