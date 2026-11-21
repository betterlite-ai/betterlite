#![cfg(test)]

use crate::limiter_records::{PROJECT_ITPM_DESCRIPTOR_KEY, PROJECT_OTPM_DESCRIPTOR_KEY};
use crate::token_reservations::tests::support::{
    NOW, ReservationFixture, START, auth, count, counter_write, read, text, tokens_key, window_key,
};

const VALUE: &str = "p1:gpt-4";

#[tokio::test]
async fn reserves_the_project_descriptors_built_from_project_metadata() {
    let fixture = ReservationFixture::at(
        &[
            (&window_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE), text(START)),
            (&tokens_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE), count(0)),
            (&window_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE), text(START)),
            (&tokens_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE), count(0)),
        ],
        NOW,
        &[("gpt-4-0613", "gpt-4")],
    );
    let auth = auth(
        r#"{"project_id": "p1", "project_metadata": {"model_itpm_limit": {"gpt-4": 500}, "model_otpm_limit": {"gpt-4": 900}}}"#,
    );

    fixture
        .reservations()
        .enforce_frame_project_io_quota(Some(&auth), Some("gpt-4-0613"), 120, 70)
        .await
        .unwrap();

    assert_eq!(
        fixture.effects.reads(),
        vec![
            read(&window_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE)),
            read(&tokens_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE)),
            read(&window_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE)),
            read(&tokens_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE)),
        ]
    );
    assert_eq!(
        fixture.effects.writes(),
        vec![
            counter_write(
                &tokens_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE),
                count(120),
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
