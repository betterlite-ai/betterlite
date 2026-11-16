#![cfg(test)]

use betterlite_config::cost_adjustments::{AdjustmentEntry, MarginFieldEntry};
use rust_decimal::Decimal;

use super::listen_services::Listener;
use super::support::{json, kwargs, logging_object, messages, spend_cost};

const CATALOG: &[(&str, &str)] = &[
    (
        "deepgram/streaming/nova-3",
        r#"{"litellm_provider": "deepgram", "input_cost_per_second": 0.00008}"#,
    ),
    (
        "deepgram/streaming/diarize",
        r#"{"litellm_provider": "deepgram", "input_cost_per_second": 0.00001}"#,
    ),
];

const TWO_SECONDS: &str = r#"[{"type": "Metadata", "duration": 2, "channels": 1}]"#;

#[tokio::test]
async fn deepgram_listen_applies_the_deepgram_discount_and_the_global_margin_to_each_registry_cost()
{
    let logging_obj = logging_object();
    let frames = messages(TWO_SECONDS);
    let caller = kwargs(&[]);
    let url = "https://api.deepgram.com/v1/listen?model=nova-3&diarize=true";
    let listener = Listener::new(CATALOG).adjusted(
        &[("deepgram", AdjustmentEntry::Percent(Decimal::new(25, 2)))],
        &[("global", AdjustmentEntry::Percent(Decimal::new(1, 1)))],
    );

    let outcome = listener
        .listen(&logging_obj, url, &frames, &caller)
        .await
        .expect("the handler runs");

    assert_eq!(spend_cost(&logging_obj), json("0.0001485"));
    assert_eq!(
        outcome.result.expect("a result").into_json(),
        json(
            r#"{"text": "",
                "_hidden_params": {"audio_transcription_duration": 2, "response_cost": 0.0001485}}"#
        )
    );
}

#[tokio::test]
async fn deepgram_listen_adds_a_fixed_margin_to_the_base_and_to_every_addon() {
    let logging_obj = logging_object();
    let frames = messages(TWO_SECONDS);
    let caller = kwargs(&[]);
    let url = "https://api.deepgram.com/v1/listen?model=nova-3&diarize=true";
    let listener = Listener::new(CATALOG).adjusted(
        &[],
        &[(
            "deepgram",
            AdjustmentEntry::Split {
                percentage: MarginFieldEntry::Absent,
                fixed_amount: MarginFieldEntry::Number(Decimal::new(1, 3)),
            },
        )],
    );

    listener
        .listen(&logging_obj, url, &frames, &caller)
        .await
        .expect("the handler runs");

    assert_eq!(spend_cost(&logging_obj), json("0.00218"));
}

#[tokio::test]
async fn deepgram_listen_prefers_the_deepgram_margin_over_the_global_one() {
    let logging_obj = logging_object();
    let frames = messages(TWO_SECONDS);
    let caller = kwargs(&[]);
    let url = "https://api.deepgram.com/v1/listen?model=nova-3";
    let listener = Listener::new(CATALOG).adjusted(
        &[],
        &[
            ("deepgram", AdjustmentEntry::Percent(Decimal::new(5, 1))),
            ("global", AdjustmentEntry::Percent(Decimal::new(1, 1))),
        ],
    );

    listener
        .listen(&logging_obj, url, &frames, &caller)
        .await
        .expect("the handler runs");

    assert_eq!(spend_cost(&logging_obj), json("0.00024"));
}

#[tokio::test]
async fn deepgram_listen_leaves_the_cost_alone_when_no_adjustment_names_deepgram() {
    let logging_obj = logging_object();
    let frames = messages(TWO_SECONDS);
    let caller = kwargs(&[]);
    let url = "https://api.deepgram.com/v1/listen?model=nova-3";
    let listener = Listener::new(CATALOG).adjusted(
        &[("openai", AdjustmentEntry::Percent(Decimal::new(5, 1)))],
        &[("openai", AdjustmentEntry::Percent(Decimal::new(5, 1)))],
    );

    listener
        .listen(&logging_obj, url, &frames, &caller)
        .await
        .expect("the handler runs");

    assert_eq!(spend_cost(&logging_obj), json("0.00016"));
}
