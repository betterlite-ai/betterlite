#![cfg(test)]

use betterlite_error::rate_limit::{RateLimitCategory, RateLimitDimension};

use crate::limiter_records::{PROJECT_ITPM_DESCRIPTOR_KEY, PROJECT_OTPM_DESCRIPTOR_KEY};
use crate::token_reservations::TokenReservationError;
use crate::token_reservations::tests::support::{
    NOW, ReservationFixture, START, auth, count, counter_write, local_write, text, tokens_key,
    window_key,
};

const VALUE: &str = "p1:gpt-4";

#[tokio::test]
async fn raises_the_429_for_a_frame_over_the_project_input_limit() {
    let fixture = ReservationFixture::at(
        &[
            (&window_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE), text(START)),
            (&tokens_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE), count(40)),
        ],
        NOW,
        &[("gpt-4-0613", "gpt-4")],
    );
    let auth = auth(
        r#"{"project_id": "p1", "project_metadata": {"model_itpm_limit": {"gpt-4": 500}, "model_otpm_limit": {"gpt-4": 900}}}"#,
    );

    let Err(TokenReservationError::RateLimited(error)) = fixture
        .reservations()
        .enforce_frame_project_io_quota(Some(&auth), Some("gpt-4-0613"), 500, 70)
        .await
    else {
        panic!("the frame over the input limit is rejected");
    };

    assert_eq!(
        error.message,
        "Rate limit exceeded for model_per_project_itpm: p1:gpt-4. Limit type: tokens. Current limit: 500, Remaining: 460. Limit resets at: 2023-11-14 22:14:20 UTC"
    );
    let headers = error
        .headers
        .as_ref()
        .expect("the rejection carries its headers");
    assert_eq!(headers["retry-after"], "60");
    assert_eq!(headers["rate_limit_type"], "tokens");
    assert_eq!(headers["reset_at"], "2023-11-14 22:14:20 UTC");
    assert_eq!(error.rate_limit_type, Some(RateLimitDimension::Tokens));
    assert_eq!(error.category, RateLimitCategory::LitellmRateLimit);
    assert_eq!(error.model, "gpt-4-0613");
    assert_eq!(error.llm_provider, "litellm_proxy");
    assert!(fixture.effects.writes().is_empty());
}

#[tokio::test]
async fn raises_the_429_for_a_frame_over_the_project_output_limit() {
    let fixture = ReservationFixture::seeded(&[
        (&window_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE), text(START)),
        (&tokens_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE), count(0)),
        (&window_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE), text(START)),
        (&tokens_key(PROJECT_OTPM_DESCRIPTOR_KEY, VALUE), count(70)),
    ]);
    let auth = auth(
        r#"{"project_id": "p1", "project_metadata": {"model_itpm_limit": {"gpt-4": 500}, "model_otpm_limit": {"gpt-4": 90}}}"#,
    );

    let Err(TokenReservationError::RateLimited(error)) = fixture
        .reservations()
        .enforce_frame_project_io_quota(Some(&auth), Some("gpt-4"), 120, 70)
        .await
    else {
        panic!("the frame over the output limit is rejected");
    };

    assert_eq!(
        error.message,
        "Rate limit exceeded for model_per_project_otpm: p1:gpt-4. Limit type: tokens. Current limit: 90, Remaining: 20. Limit resets at: 2023-11-14 22:14:20 UTC"
    );
    assert_eq!(
        fixture.effects.writes(),
        vec![
            counter_write(
                &tokens_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE),
                count(120),
                60
            ),
            local_write(&tokens_key(PROJECT_ITPM_DESCRIPTOR_KEY, VALUE), 0.0_f64, 60),
        ]
    );
}
