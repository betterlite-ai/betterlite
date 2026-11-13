use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::tiered_pricing::Scalar;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::cost::realtime::Event;
use crate::cost::realtime::error::RealtimeCostError;
use crate::cost::realtime::pricing::stream_cost;
use crate::cost::realtime::transcription::{self, TranscriptionCostRequest};

use super::support::{
    Breakdown, Catalog, cost_request, database, event, number, object, record, scalar_number,
    session_event, text, zero_usage,
};

fn billing_results(usage: Json) -> Vec<Event> {
    vec![
        session_event("transcription_session.created", Some("asr-base")),
        event(&[
            ("type", text(transcription::TRANSCRIPTION_COMPLETED_EVENT)),
            ("usage", usage),
        ]),
    ]
}

fn duration_usage() -> Json {
    object(&[("type", text("duration")), ("seconds", number("120"))])
}

fn duration_billing_results() -> Vec<Event> {
    billing_results(duration_usage())
}

fn base_duration_record() -> PriceRecord {
    record(&[("input_cost_per_second", scalar_number("0.001"))])
}

#[rstest]
#[case::base_rate(None, "0.12")]
#[case::override_rate(Some(("input_cost_per_second", "0.002")), "0.24")]
#[case::override_keeps_base_rate(Some(("input_cost_per_token", "1")), "0.12")]
#[case::explicit_zero_wins(Some(("input_cost_per_second", "0")), "0")]
fn duration_rates_prefer_the_whole_override_entry(
    #[case] override_rate: Option<(&str, &str)>,
    #[case] expected: &str,
) {
    let events = duration_billing_results();
    let usage = zero_usage();
    let mut entries = vec![("asr-base", base_duration_record())];
    let mut request = cost_request(&events, &usage, "openai", "asr-base");
    if let Some((field, rate)) = override_rate {
        entries.push(("asr-deploy", record(&[(field, scalar_number(rate))])));
        request.custom_pricing_model = Some("asr-deploy");
    }
    let catalog = Catalog::priced(&entries);
    let cost = transcription::cost(
        &TranscriptionCostRequest {
            results: &events,
            custom_llm_provider: "openai",
            litellm_model_name: "asr-base",
            custom_pricing_model: request.custom_pricing_model,
        },
        &catalog,
    )
    .expect("the duration is priced");
    assert_eq!(
        cost,
        Decimal::from_str(expected).expect("the expected decimal")
    );
}

fn token_mode_usage() -> Json {
    object(&[
        ("type", text("tokens")),
        (
            "input_token_details",
            object(&[
                ("audio_tokens", number("100")),
                ("text_tokens", number("12")),
            ]),
        ),
        ("output_tokens", number("30")),
    ])
}

fn token_results() -> Vec<Event> {
    billing_results(token_mode_usage())
}

fn base_token_record() -> PriceRecord {
    record(&[
        ("input_cost_per_audio_token", scalar_number("0.000006")),
        ("input_cost_per_token", scalar_number("0.0000025")),
        ("output_cost_per_token", scalar_number("0.00001")),
    ])
}

fn transcription_cost_with(entry: Option<PriceRecord>) -> Decimal {
    let events = token_results();
    let mut entries = vec![("asr-base", base_token_record())];
    let custom_pricing_model = entry.map(|found| {
        entries.push(("asr-deploy", found));
        "asr-deploy"
    });
    let catalog = Catalog::priced(&entries);
    transcription::cost(
        &TranscriptionCostRequest {
            results: &events,
            custom_llm_provider: "openai",
            litellm_model_name: "asr-base",
            custom_pricing_model,
        },
        &catalog,
    )
    .expect("the tokens are priced")
}

#[test]
fn token_rates_bill_audio_text_and_output() {
    assert_eq!(
        transcription_cost_with(None),
        Decimal::from_str("0.00093").expect("a decimal")
    );
}

#[test]
fn an_override_entry_prices_every_family_before_the_base() {
    let override_entry = record(&[
        ("input_cost_per_token", scalar_number("0")),
        ("output_cost_per_token", scalar_number("0")),
    ]);
    assert_eq!(transcription_cost_with(Some(override_entry)), Decimal::ZERO);
}

#[test]
fn an_override_audio_zero_still_reads_base_text_and_output_rates() {
    let override_entry = record(&[("input_cost_per_audio_token", scalar_number("0"))]);
    assert_eq!(
        transcription_cost_with(Some(override_entry)),
        Decimal::from_str("0.00033").expect("a decimal")
    );
}

#[test]
fn an_override_without_token_rates_keeps_the_base_rates() {
    let override_entry = record(&[("input_cost_per_second", scalar_number("0.001"))]);
    assert_eq!(
        transcription_cost_with(Some(override_entry)),
        Decimal::from_str("0.00093").expect("a decimal")
    );
}

#[test]
fn a_rate_the_raw_entry_does_not_declare_does_not_qualify() {
    let events = token_results();
    let mut catalog = Catalog::priced(&[("asr-base", base_token_record())]);
    catalog.raw.insert(
        "asr-deploy".to_owned(),
        record(&[("mode", Scalar::Text("audio_transcription".to_owned()))]),
    );
    let mut effective = record(&[
        ("input_cost_per_token", scalar_number("0.5")),
        ("output_cost_per_token", scalar_number("0.5")),
    ]);
    effective.insert("key".to_owned(), Scalar::Text("asr-deploy".to_owned()));
    catalog.effective.insert("asr-deploy".to_owned(), effective);
    let cost = transcription::cost(
        &TranscriptionCostRequest {
            results: &events,
            custom_llm_provider: "openai",
            litellm_model_name: "asr-base",
            custom_pricing_model: Some("asr-deploy"),
        },
        &catalog,
    )
    .expect("the base rates apply");
    assert_eq!(cost, Decimal::from_str("0.00093").expect("a decimal"));
}

#[test]
fn token_usage_with_only_input_tokens_bills_zero() {
    let events = [
        session_event("transcription_session.created", Some("asr-base")),
        event(&[
            ("type", text(transcription::TRANSCRIPTION_COMPLETED_EVENT)),
            (
                "usage",
                object(&[("type", text("tokens")), ("input_tokens", number("500"))]),
            ),
        ]),
    ];
    let catalog = Catalog::priced(&[("asr-base", base_token_record())]);
    let cost = transcription::cost(
        &TranscriptionCostRequest {
            results: &events,
            custom_llm_provider: "openai",
            litellm_model_name: "asr-base",
            custom_pricing_model: None,
        },
        &catalog,
    )
    .expect("aggregate input tokens are not billed");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn unknown_or_missing_usage_types_bill_zero() {
    let events = [
        event(&[
            ("type", text(transcription::TRANSCRIPTION_COMPLETED_EVENT)),
            ("usage", object(&[("type", text("future_billing_type"))])),
        ]),
        event(&[
            ("type", text(transcription::TRANSCRIPTION_COMPLETED_EVENT)),
            ("usage", object(&[])),
        ]),
        event(&[
            ("type", text(transcription::TRANSCRIPTION_COMPLETED_EVENT)),
            (
                "usage",
                object(&[
                    ("type", text("future_billing_type")),
                    (
                        "input_token_details",
                        object(&[("audio_tokens", number("10"))]),
                    ),
                    ("output_tokens", number("5")),
                ]),
            ),
        ]),
        event(&[
            ("type", text(transcription::TRANSCRIPTION_COMPLETED_EVENT)),
            ("usage", Json::Null),
        ]),
    ];
    let catalog = Catalog::priced(&[("asr-base", base_token_record())]);
    let cost = transcription::cost(
        &TranscriptionCostRequest {
            results: &events,
            custom_llm_provider: "openai",
            litellm_model_name: "asr-base",
            custom_pricing_model: None,
        },
        &catalog,
    )
    .expect("unknown usage types are zero");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn missing_model_entries_bill_zero() {
    let events = duration_billing_results();
    let cost = transcription::cost(
        &TranscriptionCostRequest {
            results: &events,
            custom_llm_provider: "openai",
            litellm_model_name: "asr-base",
            custom_pricing_model: None,
        },
        &Catalog::default(),
    )
    .expect("no entry bills zero");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn no_completed_event_skips_model_resolution() {
    let events = [event(&[
        ("type", text("session.created")),
        ("session", text("not-an-object")),
    ])];
    let cost = transcription::cost(
        &TranscriptionCostRequest {
            results: &events,
            custom_llm_provider: "openai",
            litellm_model_name: "asr-base",
            custom_pricing_model: None,
        },
        &Catalog::default(),
    )
    .expect("no completed event bills zero");
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn overflow_in_realtime_money_is_a_typed_failure() {
    let events = [
        session_event("session.created", Some("asr-model")),
        event(&[
            ("type", text(transcription::TRANSCRIPTION_COMPLETED_EVENT)),
            (
                "usage",
                object(&[("type", text("duration")), ("seconds", number("2"))]),
            ),
        ]),
    ];
    let entry = record(&[(
        "input_cost_per_second",
        scalar_number("79228162514264337593543950335"),
    )]);
    let catalog = Catalog::priced(&[("asr-model", entry)]);
    let usage = zero_usage();
    let request = cost_request(&events, &usage, "openai", "asr-model");
    let error = stream_cost(&request, &catalog, &database(), None::<&mut Breakdown>)
        .expect_err("the product does not fit");
    assert!(matches!(error, RealtimeCostError::Overflow));
}

fn nested_session(
    fields: &[(&str, Json)],
    legacy: Option<&str>,
    session_model: Option<&str>,
) -> Json {
    let mut session = event(fields);
    if let Some(found) = legacy {
        session.insert(
            "input_audio_transcription".to_owned(),
            object(&[("model", text(found))]),
        );
    }
    if let Some(found) = session_model {
        session.insert("model".to_owned(), text(found));
    }
    Json::Object(session)
}

#[test]
fn the_nested_transcription_model_wins_over_the_legacy_object() {
    let nested = object(&[("model", text("nested-asr"))]);
    let events = [event(&[
        ("type", text("session.created")),
        (
            "session",
            nested_session(
                &[(
                    "audio",
                    object(&[("input", object(&[("transcription", nested)]))]),
                )],
                Some("legacy-asr"),
                None,
            ),
        ),
    ])];
    assert_eq!(
        transcription::session::model_name(&events).expect("the session is readable"),
        Some("nested-asr".to_owned())
    );
}

#[test]
fn a_truthy_nested_object_without_a_model_blocks_the_legacy_object() {
    let nested = object(&[("language", text("en"))]);
    let events = [event(&[
        ("type", text("session.updated")),
        (
            "session",
            nested_session(
                &[(
                    "audio",
                    object(&[("input", object(&[("transcription", nested)]))]),
                )],
                Some("legacy-asr"),
                Some("session-model"),
            ),
        ),
    ])];
    assert_eq!(
        transcription::session::model_name(&events).expect("the session is readable"),
        Some("session-model".to_owned())
    );
}

#[test]
fn the_legacy_object_applies_when_the_nested_transcription_is_absent() {
    let events = [event(&[
        ("type", text("transcription_session.created")),
        ("session", nested_session(&[], Some("legacy-asr"), None)),
    ])];
    assert_eq!(
        transcription::session::model_name(&events).expect("the session is readable"),
        Some("legacy-asr".to_owned())
    );
}

#[test]
fn an_empty_nested_object_falls_back_to_the_legacy_object() {
    let events = [event(&[
        ("type", text("transcription_session.created")),
        (
            "session",
            nested_session(
                &[(
                    "audio",
                    object(&[("input", object(&[("transcription", object(&[]))]))]),
                )],
                Some("legacy-asr"),
                None,
            ),
        ),
    ])];
    assert_eq!(
        transcription::session::model_name(&events).expect("the session is readable"),
        Some("legacy-asr".to_owned())
    );
}

#[test]
fn a_blocked_event_lets_the_next_session_event_supply_the_model() {
    let blocked = nested_session(
        &[(
            "audio",
            object(&[(
                "input",
                object(&[("transcription", object(&[("language", text("en"))]))]),
            )]),
        )],
        Some("legacy-asr"),
        None,
    );
    let events = [
        event(&[("type", text("session.created")), ("session", blocked)]),
        session_event("session.created", Some("second-model")),
    ];
    assert_eq!(
        transcription::session::model_name(&events).expect("the events are readable"),
        Some("second-model".to_owned())
    );
}

#[test]
fn no_event_model_returns_none() {
    let events = [event(&[("type", text("response.done"))])];
    assert_eq!(
        transcription::session::model_name(&events).expect("the events are readable"),
        None
    );
}

#[test]
fn a_malformed_session_is_a_typed_failure() {
    let events = [event(&[
        ("type", text("session.created")),
        ("session", text("not-an-object")),
    ])];
    let error = transcription::session::model_name(&events).expect_err("session must be an object");
    assert!(matches!(
        error,
        RealtimeCostError::Shape {
            field: "session",
            ..
        }
    ));
}

#[test]
fn an_empty_model_string_is_not_a_candidate() {
    let events = [event(&[
        ("type", text("session.created")),
        ("session", object(&[("model", text(""))])),
    ])];
    assert_eq!(
        transcription::session::model_name(&events).expect("an empty model is absent"),
        None
    );
}

#[test]
fn a_null_raw_rate_does_not_qualify() {
    let events = duration_billing_results();
    let mut catalog = Catalog::priced(&[("asr-base", base_duration_record())]);
    catalog.raw.insert(
        "asr-deploy".to_owned(),
        record(&[("input_cost_per_second", Scalar::Null)]),
    );
    let mut effective = record(&[("input_cost_per_second", scalar_number("0.002"))]);
    effective.insert("key".to_owned(), Scalar::Text("asr-deploy".to_owned()));
    catalog.effective.insert("asr-deploy".to_owned(), effective);
    let cost = transcription::cost(
        &TranscriptionCostRequest {
            results: &events,
            custom_llm_provider: "openai",
            litellm_model_name: "asr-base",
            custom_pricing_model: Some("asr-deploy"),
        },
        &catalog,
    )
    .expect("the base rate applies");
    assert_eq!(cost, Decimal::from_str("0.12").expect("a decimal"));
}

#[test]
fn a_text_rate_is_a_typed_failure() {
    let events = duration_billing_results();
    let entry = record(&[("input_cost_per_second", Scalar::Text("free".to_owned()))]);
    let catalog = Catalog::priced(&[("asr-base", entry)]);
    let error = transcription::cost(
        &TranscriptionCostRequest {
            results: &events,
            custom_llm_provider: "openai",
            litellm_model_name: "asr-base",
            custom_pricing_model: None,
        },
        &catalog,
    )
    .expect_err("a text rate is not a number");
    assert!(matches!(
        error,
        RealtimeCostError::NotANumber {
            field: "input_cost_per_second"
        }
    ));
}

#[test]
fn the_legacy_object_applies_when_audio_has_no_input() {
    let events = [event(&[
        ("type", text("transcription_session.created")),
        (
            "session",
            nested_session(&[("audio", object(&[]))], Some("legacy-asr"), None),
        ),
    ])];
    assert_eq!(
        transcription::session::model_name(&events).expect("the session is readable"),
        Some("legacy-asr".to_owned())
    );
}

#[test]
fn the_legacy_object_applies_when_audio_is_null() {
    let events = [event(&[
        ("type", text("transcription_session.created")),
        (
            "session",
            nested_session(&[("audio", Json::Null)], Some("legacy-asr"), None),
        ),
    ])];
    assert_eq!(
        transcription::session::model_name(&events).expect("the session is readable"),
        Some("legacy-asr".to_owned())
    );
}
