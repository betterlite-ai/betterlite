#![cfg(test)]

use betterlite_core::api::media_relay_logging::deepgram::DeepgramListenRequest;
use betterlite_core::api::media_relay_logging::error::MediaRelayLoggingError;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_models::decisions::Json;

use super::deepgram::deepgram_listen_passthrough_handler;
use super::support::{
    Registry, json, kwargs, logged_model, logging_object, lookup, messages, registry, spend,
    spend_cost, spend_row,
};

const CATALOG: &[(&str, &str)] = &[
    (
        "deepgram/streaming/nova-3",
        r#"{"litellm_provider": "deepgram", "input_cost_per_second": 0.00008}"#,
    ),
    (
        "deepgram/streaming/nova-3-multilingual",
        r#"{"litellm_provider": "deepgram", "input_cost_per_second": 0.0001}"#,
    ),
    (
        "deepgram/streaming/redact",
        r#"{"litellm_provider": "deepgram", "input_cost_per_second": 0.00003333}"#,
    ),
    (
        "deepgram/streaming/diarize",
        r#"{"litellm_provider": "deepgram", "input_cost_per_second": 0.00001}"#,
    ),
];

const TWO_CHANNEL: &str = r#"[
    {"type": "Metadata", "duration": 2.0, "channels": 2},
    {"type": "Results", "is_final": true, "channel": {"alternatives": [{"transcript": "hello"}]}},
    {"type": "Results", "is_final": true, "channel": {"alternatives": [{"transcript": "world"}]}}
]"#;

fn request<'request>(
    registry: &'request Registry,
    url: &'request str,
    frames: &'request [Json],
    caller: &'request betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs,
) -> DeepgramListenRequest<'request> {
    DeepgramListenRequest {
        upstream_url: url,
        messages: frames,
        kwargs: caller,
        catalog: &registry.catalog,
        lookup: &registry.lookup,
    }
}

#[test]
fn deepgram_listen_bills_the_base_and_each_priced_addon() {
    let registry = registry(CATALOG);
    let logging_obj = logging_object();
    let frames = messages(TWO_CHANNEL);
    let caller = kwargs(&[
        ("custom", json("1")),
        ("model", json("stale")),
        ("response_cost", json("9")),
    ]);
    let url = "https://api.deepgram.com/v1/listen?model=nova-3&redact=pci&diarize=true";

    let outcome = deepgram_listen_passthrough_handler(
        &logging_obj,
        &request(&registry, url, &frames, &caller),
    )
    .expect("the handler runs");

    assert_eq!(logged_model(&logging_obj), "nova-3");
    let cost = json("0.00049332");
    assert_eq!(
        spend(&logging_obj),
        spend_row("nova-3", "deepgram", cost.clone())
    );
    let mut expected_kwargs = spend_row("nova-3", "deepgram", cost);
    expected_kwargs.insert("custom".to_owned(), json("1"));
    assert_eq!(outcome.kwargs.entries, expected_kwargs);
    assert!(matches!(
        &outcome.result,
        Some(PassThroughResponse::Transcription(_))
    ));
    assert_eq!(
        outcome.result.expect("a result").into_json(),
        json(
            r#"{"text": "hello world",
                "_hidden_params": {"audio_transcription_duration": 4, "response_cost": 0.00049332}}"#
        )
    );
    assert_eq!(caller.entries.len(), 3);
}

#[test]
fn deepgram_listen_drops_an_addon_without_a_registry_price() {
    let registry = registry(CATALOG);
    let logging_obj = logging_object();
    let frames = messages(TWO_CHANNEL);
    let caller = kwargs(&[]);
    let url = "https://api.deepgram.com/v1/listen?model=nova-3&redact=pci&keyterm=abc";

    let outcome = deepgram_listen_passthrough_handler(
        &logging_obj,
        &request(&registry, url, &frames, &caller),
    )
    .expect("the handler runs");

    assert_eq!(spend_cost(&logging_obj), json("0.00045332"));
    assert_eq!(
        outcome.result.expect("a result").into_json(),
        json(
            r#"{"text": "hello world",
                "_hidden_params": {"audio_transcription_duration": 4, "response_cost": 0.00045332}}"#
        )
    );
}

#[test]
fn deepgram_listen_unpriced_base_keeps_the_spend_row() {
    let registry = registry(&[(
        "deepgram/streaming/redact",
        r#"{"litellm_provider": "deepgram", "input_cost_per_second": 0.00003333}"#,
    )]);
    let logging_obj = logging_object();
    let frames = messages(TWO_CHANNEL);
    let caller = kwargs(&[]);
    let url = "https://api.deepgram.com/v1/listen?model=nova-9&redact=pci";

    let outcome = deepgram_listen_passthrough_handler(
        &logging_obj,
        &request(&registry, url, &frames, &caller),
    )
    .expect("the handler runs");

    assert_eq!(logged_model(&logging_obj), "nova-9");
    assert_eq!(
        spend(&logging_obj),
        spend_row("nova-9", "deepgram", Json::Null)
    );
    assert_eq!(
        outcome.kwargs.entries,
        spend_row("nova-9", "deepgram", Json::Null)
    );
    assert_eq!(
        outcome.result.expect("a result").into_json(),
        json(
            r#"{"text": "hello world",
                "_hidden_params": {"audio_transcription_duration": 4, "response_cost": null}}"#
        )
    );
}

#[test]
fn deepgram_listen_zero_length_session_costs_zero() {
    let registry = registry(CATALOG);
    let logging_obj = logging_object();
    let frames = messages("[]");
    let caller = kwargs(&[]);
    let url = "https://api.deepgram.com/v1/listen?model=nova-3";

    let outcome = deepgram_listen_passthrough_handler(
        &logging_obj,
        &request(&registry, url, &frames, &caller),
    )
    .expect("the handler runs");

    assert_eq!(spend_cost(&logging_obj), json("0"));
    assert_eq!(
        outcome.result.expect("a result").into_json(),
        json(
            r#"{"text": "", "_hidden_params": {"audio_transcription_duration": 0, "response_cost": 0}}"#
        )
    );
}

#[test]
fn deepgram_listen_declared_channels_multiply_the_billed_seconds() {
    let registry = registry(CATALOG);
    let logging_obj = logging_object();
    let frames = messages(r#"[{"type": "Metadata", "duration": 1}]"#);
    let caller = kwargs(&[]);
    let url = "https://api.deepgram.com/v1/listen?model=nova-3&channels=3";

    let outcome = deepgram_listen_passthrough_handler(
        &logging_obj,
        &request(&registry, url, &frames, &caller),
    )
    .expect("the handler runs");

    assert_eq!(spend_cost(&logging_obj), json("0.00024"));
    assert_eq!(
        outcome.result.expect("a result").into_json(),
        json(
            r#"{"text": "", "_hidden_params": {"audio_transcription_duration": 3, "response_cost": 0.00024}}"#
        )
    );
}

#[test]
fn deepgram_listen_multilingual_language_prices_the_multilingual_entry() {
    let registry = registry(CATALOG);
    let logging_obj = logging_object();
    let frames = messages(r#"[{"type": "Metadata", "duration": 2}]"#);
    let caller = kwargs(&[]);
    let url = "https://api.deepgram.com/v1/listen?model=nova-3&language=multi";

    deepgram_listen_passthrough_handler(&logging_obj, &request(&registry, url, &frames, &caller))
        .expect("the handler runs");

    assert_eq!(spend_cost(&logging_obj), json("0.0002"));
}

#[test]
fn deepgram_listen_skips_messages_that_are_not_listen_frames() {
    let registry = registry(CATALOG);
    let logging_obj = logging_object();
    let frames = messages(
        r#"[
            {"type": "Metadata", "duration": 1, "channels": 1},
            {"type": "UtteranceEnd", "last_word_end": 1},
            {"type": "Results", "is_final": false, "channel": {"alternatives": [{"transcript": "partial"}]}},
            {"type": "Results", "is_final": true, "channel": {"alternatives": [{"transcript": "hello"}]}},
            {"channels": 7},
            "not a frame"
        ]"#,
    );
    let caller = kwargs(&[]);
    let url = "https://api.deepgram.com/v1/listen?model=nova-3";

    let outcome = deepgram_listen_passthrough_handler(
        &logging_obj,
        &request(&registry, url, &frames, &caller),
    )
    .expect("the handler runs");

    assert_eq!(spend_cost(&logging_obj), json("0.00008"));
    assert_eq!(
        outcome.result.expect("a result").into_json(),
        json(
            r#"{"text": "hello", "_hidden_params": {"audio_transcription_duration": 1, "response_cost": 0.00008}}"#
        )
    );
}

#[test]
fn deepgram_listen_unreadable_base_rate_leaves_the_cost_absent() {
    let registry = Registry {
        catalog: registry(CATALOG).catalog,
        lookup: lookup(&[(
            "deepgram/streaming/nova-3",
            r#"{"litellm_provider": "deepgram", "input_cost_per_second": [1]}"#,
        )]),
    };
    let logging_obj = logging_object();
    let frames = messages(TWO_CHANNEL);
    let caller = kwargs(&[]);
    let url = "https://api.deepgram.com/v1/listen?model=nova-3";

    let outcome = deepgram_listen_passthrough_handler(
        &logging_obj,
        &request(&registry, url, &frames, &caller),
    )
    .expect("the handler runs");

    assert_eq!(spend_cost(&logging_obj), Json::Null);
    assert_eq!(
        outcome.result.expect("a result").into_json(),
        json(
            r#"{"text": "hello world",
                "_hidden_params": {"audio_transcription_duration": 4, "response_cost": null}}"#
        )
    );
}

#[test]
fn deepgram_listen_price_sum_beyond_the_decimal_range_leaves_the_cost_absent() {
    let registry = Registry {
        catalog: registry(&[(
            "deepgram/streaming/nova-3",
            r#"{"litellm_provider": "deepgram", "input_cost_per_second": 0.00008}"#,
        )])
        .catalog,
        lookup: lookup(&[
            (
                "deepgram/streaming/nova-3",
                r#"{"litellm_provider": "deepgram", "input_cost_per_second": 4e28}"#,
            ),
            (
                "deepgram/streaming/redact",
                r#"{"litellm_provider": "deepgram", "input_cost_per_second": 4e28}"#,
            ),
        ]),
    };
    let logging_obj = logging_object();
    let frames = messages(r#"[{"type": "Metadata", "duration": 1}]"#);
    let caller = kwargs(&[]);
    let url = "https://api.deepgram.com/v1/listen?model=nova-3&redact=pci";
    let outcome = deepgram_listen_passthrough_handler(
        &logging_obj,
        &request(&registry, url, &frames, &caller),
    )
    .expect("the handler runs");

    assert_eq!(spend_cost(&logging_obj), Json::Null);
    assert_eq!(
        outcome.result.expect("a result").into_json(),
        json(
            r#"{"text": "", "_hidden_params": {"audio_transcription_duration": 1, "response_cost": null}}"#
        )
    );
}

#[test]
fn deepgram_listen_invalid_upstream_url_is_a_target_error() {
    let registry = registry(CATALOG);
    let logging_obj = logging_object();
    let frames = messages(TWO_CHANNEL);
    let caller = kwargs(&[]);

    let error = deepgram_listen_passthrough_handler(
        &logging_obj,
        &request(&registry, "not a url", &frames, &caller),
    )
    .expect_err("the target cannot be read");

    assert!(matches!(error, MediaRelayLoggingError::DeepgramTarget(_)));
}

#[test]
fn deepgram_listen_duration_beyond_the_decimal_range_is_an_overflow() {
    let registry = registry(CATALOG);
    let logging_obj = logging_object();
    let frames = messages(r#"[{"type": "Metadata", "duration": 7.9e28, "channels": 2}]"#);
    let caller = kwargs(&[]);
    let url = "https://api.deepgram.com/v1/listen?model=nova-3";

    let error = deepgram_listen_passthrough_handler(
        &logging_obj,
        &request(&registry, url, &frames, &caller),
    )
    .expect_err("the billed duration exceeds the decimal range");

    assert!(matches!(error, MediaRelayLoggingError::Overflow));
}
