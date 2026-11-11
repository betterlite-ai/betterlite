use std::collections::BTreeMap;

use betterlite_models::tiered_pricing::Scalar;
use betterlite_models::usage_sum::{PromptTokensDetails, Usage};
use rust_decimal::Decimal;

use crate::cost::realtime::error::RealtimeCostError;
use crate::cost::realtime::pricing::stream_cost;
use crate::cost::realtime::transcription;

use super::support::{
    Breakdown, Catalog, cost_request, database, event, number, object, priced, record,
    scalar_number, session_event, text, token_usage, zero_usage,
};

#[test]
fn candidate_names_follow_custom_base_sessions_request_order() {
    let events = [
        session_event("session.created", Some("session-one")),
        session_event("session.created", None),
        session_event("session.created", Some("session-two")),
        session_event("session.created", Some("session-one")),
    ];
    let usage = zero_usage();
    let mut request = cost_request(&events, &usage, "openai", "request-model");
    request.custom_pricing_model = Some("custom-model");
    request.base_pricing_model = Some("base-model");
    let lookup = Catalog::default();
    let total = stream_cost(&request, &lookup, &database(), None::<&mut Breakdown>)
        .expect("unpriced candidates bill zero");
    assert_eq!(total, Decimal::ZERO);
    assert_eq!(
        lookup.calls(),
        [
            "custom-model",
            "base-model",
            "session-one",
            "session-two",
            "session-one",
            "request-model"
        ]
    );
}

#[test]
fn an_empty_candidate_string_is_still_tried() {
    let events = [session_event("session.created", Some(""))];
    let usage = zero_usage();
    let request = cost_request(&events, &usage, "openai", "request-model");
    let lookup = Catalog::default();
    stream_cost(&request, &lookup, &database(), None::<&mut Breakdown>)
        .expect("unpriced candidates bill zero");
    assert_eq!(lookup.calls(), ["", "request-model"]);
}

#[test]
fn a_session_created_event_requires_its_session() {
    let events = [event(&[("type", text("session.created"))])];
    let usage = zero_usage();
    let request = cost_request(&events, &usage, "openai", "request-model");
    let error = stream_cost(
        &request,
        &Catalog::default(),
        &database(),
        None::<&mut Breakdown>,
    )
    .expect_err("session is required");
    assert!(matches!(
        error,
        RealtimeCostError::MissingField { field: "session" }
    ));
}

#[test]
fn the_first_priced_candidate_wins() {
    let events = [
        session_event("session.created", Some("first")),
        session_event("session.created", Some("second")),
    ];
    let catalog = Catalog::priced(&[
        ("first", priced("0.001", "0.002")),
        ("second", priced("1", "1")),
    ]);
    let usage = token_usage(100, 10);
    let request = cost_request(&events, &usage, "openai", "second");
    let lookup = catalog;
    let total = stream_cost(&request, &lookup, &database(), None::<&mut Breakdown>)
        .expect("the first candidate is priced");
    assert_eq!(total, Decimal::new(12, 2));
    assert_eq!(lookup.calls(), ["first"]);
}

#[test]
fn a_failed_candidate_falls_through_to_the_next() {
    let events = [session_event("session.created", Some("missing"))];
    let catalog = Catalog::priced(&[("request-model", priced("0.001", "0.002"))]);
    let usage = token_usage(100, 10);
    let request = cost_request(&events, &usage, "openai", "request-model");
    let lookup = catalog;
    let total = stream_cost(&request, &lookup, &database(), None::<&mut Breakdown>)
        .expect("the later candidate is priced");
    assert_eq!(total, Decimal::new(12, 2));
    assert_eq!(lookup.calls(), ["missing", "request-model"]);
}

#[test]
fn a_declared_free_entry_is_accepted() {
    let events = [session_event("session.created", Some("free"))];
    let mut free = record(&[("litellm_provider", Scalar::Text("openai".to_owned()))]);
    free.extend(priced("0", "0"));
    let catalog = Catalog::priced(&[("free", free), ("paid", priced("1", "1"))]);
    let usage = token_usage(100, 10);
    let request = cost_request(&events, &usage, "openai", "paid");
    let lookup = catalog;
    let total = stream_cost(&request, &lookup, &database(), None::<&mut Breakdown>)
        .expect("the free entry is accepted");
    assert_eq!(total, Decimal::ZERO);
    assert_eq!(lookup.calls(), ["free"]);
}

#[test]
fn synthesized_zero_rates_without_a_raw_declaration_fall_through() {
    let events = [session_event("session.created", Some("synth"))];
    let mut catalog = Catalog::priced(&[("paid", priced("0.001", "0.002"))]);
    catalog.raw.insert(
        "synth".to_owned(),
        record(&[("mode", Scalar::Text("realtime".to_owned()))]),
    );
    let mut synthesized = priced("0", "0");
    synthesized.insert("key".to_owned(), Scalar::Text("synth".to_owned()));
    catalog.effective.insert("synth".to_owned(), synthesized);
    let usage = token_usage(100, 10);
    let request = cost_request(&events, &usage, "openai", "paid");
    let lookup = catalog;
    let total = stream_cost(&request, &lookup, &database(), None::<&mut Breakdown>)
        .expect("the priced candidate wins");
    assert_eq!(total, Decimal::new(12, 2));
    assert_eq!(lookup.calls(), ["synth", "paid"]);
}

#[test]
fn a_per_image_only_entry_does_not_declare_pricing() {
    let events = [session_event("session.created", Some("image-only"))];
    let mut catalog = Catalog::priced(&[("paid", priced("0.001", "0.002"))]);
    catalog.raw.insert(
        "image-only".to_owned(),
        record(&[("input_cost_per_image", scalar_number("0"))]),
    );
    let mut synthesized = priced("0", "0");
    synthesized.insert("key".to_owned(), Scalar::Text("image-only".to_owned()));
    catalog
        .effective
        .insert("image-only".to_owned(), synthesized);
    let usage = token_usage(100, 10);
    let request = cost_request(&events, &usage, "openai", "paid");
    let lookup = catalog;
    let total = stream_cost(&request, &lookup, &database(), None::<&mut Breakdown>)
        .expect("the priced candidate wins");
    assert_eq!(total, Decimal::new(12, 2));
    assert_eq!(lookup.calls(), ["image-only", "paid"]);
}

#[test]
fn a_tiered_pricing_declaration_accepts_a_zero_candidate() {
    let events = [session_event("session.created", Some("tiered"))];
    let mut catalog = Catalog::priced(&[("paid", priced("1", "1"))]);
    catalog.raw.insert(
        "tiered".to_owned(),
        record(&[("tiered_pricing", Scalar::Array(Vec::new()))]),
    );
    catalog
        .effective
        .insert("tiered".to_owned(), priced("0", "0"));
    let usage = token_usage(100, 10);
    let request = cost_request(&events, &usage, "openai", "paid");
    let lookup = catalog;
    let total = stream_cost(&request, &lookup, &database(), None::<&mut Breakdown>)
        .expect("the tiered declaration accepts zero");
    assert_eq!(total, Decimal::ZERO);
    assert_eq!(lookup.calls(), ["tiered"]);
}

#[test]
fn the_provider_scoped_raw_entry_can_declare_pricing() {
    let events = [session_event("session.created", Some("deployment"))];
    let mut catalog = Catalog::default();
    catalog
        .raw
        .insert("openai/deployment".to_owned(), priced("0", "0"));
    catalog
        .effective
        .insert("deployment".to_owned(), priced("0", "0"));
    let usage = token_usage(0, 0);
    let request = cost_request(&events, &usage, "openai", "request-model");
    let lookup = catalog;
    let total = stream_cost(&request, &lookup, &database(), None::<&mut Breakdown>)
        .expect("the provider-scoped entry declares pricing");
    assert_eq!(total, Decimal::ZERO);
    assert_eq!(lookup.calls(), ["deployment"]);
}

#[test]
fn residency_multiplies_token_costs() {
    let events = [session_event("session.created", Some("regional"))];
    let mut entry = priced("0.001", "0.002");
    entry.insert(
        "regional_processing_uplift_multiplier_eu".to_owned(),
        scalar_number("2"),
    );
    let catalog = Catalog::priced(&[("regional", entry)]);
    let usage = token_usage(100, 10);
    let mut request = cost_request(&events, &usage, "openai", "regional");
    request.data_residency = Some("eu");
    let mut breakdown = Breakdown::default();
    let total = stream_cost(&request, &catalog, &database(), Some(&mut breakdown))
        .expect("the candidate is priced");
    assert_eq!(total, Decimal::new(24, 2));
    assert_eq!(breakdown.data_residency, Some("eu".to_owned()));
    assert_eq!(breakdown.input_cost, Some(Decimal::new(2, 1)));
    assert_eq!(breakdown.output_cost, Some(Decimal::new(4, 2)));
}

#[test]
fn transcription_cost_joins_the_total_and_the_breakdown() {
    let events = [
        session_event("session.created", Some("asr-model")),
        event(&[
            ("type", text(transcription::TRANSCRIPTION_COMPLETED_EVENT)),
            (
                "usage",
                object(&[("type", text("duration")), ("seconds", number("120"))]),
            ),
        ]),
    ];
    let mut entry = record(&[("input_cost_per_second", scalar_number("0.001"))]);
    entry.insert(
        "regional_processing_uplift_multiplier_eu".to_owned(),
        scalar_number("5"),
    );
    let catalog = Catalog::priced(&[("asr-model", entry)]);
    let usage = zero_usage();
    let mut request = cost_request(&events, &usage, "openai", "asr-model");
    request.data_residency = Some("eu");
    let mut breakdown = Breakdown::default();
    let total = stream_cost(&request, &catalog, &database(), Some(&mut breakdown))
        .expect("the session is priced");
    assert_eq!(total, Decimal::new(12, 2));
    assert_eq!(breakdown.input_cost, Some(Decimal::ZERO));
    assert_eq!(breakdown.output_cost, Some(Decimal::ZERO));
    assert_eq!(breakdown.tool_usage_cost, Some(Decimal::ZERO));
    assert_eq!(breakdown.total_cost, Some(Decimal::new(12, 2)));
    assert_eq!(
        breakdown.additional_costs,
        Some(BTreeMap::from([(
            "transcription_cost".to_owned(),
            Decimal::new(12, 2)
        )]))
    );
    assert_eq!(breakdown.service_tier, None);
    assert_eq!(breakdown.data_residency, Some("eu".to_owned()));
}

#[test]
fn a_zero_transcription_charge_adds_no_breakdown_line() {
    let events = [event(&[
        ("type", text(transcription::TRANSCRIPTION_COMPLETED_EVENT)),
        (
            "usage",
            object(&[("type", text("duration")), ("seconds", number("120"))]),
        ),
    ])];
    let usage = zero_usage();
    let request = cost_request(&events, &usage, "openai", "unpriced");
    let mut breakdown = Breakdown::default();
    let total = stream_cost(
        &request,
        &Catalog::default(),
        &database(),
        Some(&mut breakdown),
    )
    .expect("an unpriced session bills zero");
    assert_eq!(total, Decimal::ZERO);
    assert_eq!(breakdown.additional_costs, None);
    assert_eq!(breakdown.total_cost, Some(Decimal::ZERO));
}

#[test]
fn a_missing_breakdown_target_does_not_fail_the_charge() {
    let events = [session_event("session.created", Some("priced"))];
    let catalog = Catalog::priced(&[("priced", priced("0.001", "0.002"))]);
    let usage = token_usage(100, 10);
    let request = cost_request(&events, &usage, "openai", "priced");
    let total = stream_cost(&request, &catalog, &database(), None::<&mut Breakdown>)
        .expect("the charge survives a missing target");
    assert_eq!(total, Decimal::new(12, 2));
}

fn cache_usage(prompt: i64, text: i64, cache_write: i64) -> Usage {
    Usage {
        prompt_tokens: prompt,
        completion_tokens: 0,
        total_tokens: prompt,
        prompt_tokens_details: Some(PromptTokensDetails {
            text_tokens: Some(text),
            audio_tokens: None,
            image_tokens: None,
            video_tokens: None,
            web_search_requests: None,
            google_maps_grounding_requests: None,
            tool_use_tokens: None,
            character_count: None,
            image_count: None,
            video_length_seconds: None,
            audio_length_seconds: None,
            query_count: None,
            cache_write_tokens: Some(cache_write),
            cache_creation_tokens: Some(cache_write),
            cached_tokens: Some(0),
            cached_tokens_details: None,
        }),
        completion_tokens_details: None,
        numeric_extras: BTreeMap::new(),
    }
}

#[test]
fn a_positive_sum_needs_no_declaration() {
    let events = [session_event("session.created", Some("floating"))];
    let mut catalog = Catalog::default();
    catalog
        .effective
        .insert("floating".to_owned(), priced("0.001", "0.002"));
    let usage = token_usage(100, 10);
    let request = cost_request(&events, &usage, "openai", "floating");
    let total = stream_cost(&request, &catalog, &database(), None::<&mut Breakdown>)
        .expect("a positive sum is accepted");
    assert_eq!(total, Decimal::new(12, 2));
    assert_eq!(catalog.calls(), ["floating"]);
}

#[test]
fn a_resolved_catalog_key_can_declare_pricing() {
    let events = [session_event("session.created", Some("alias"))];
    let mut catalog = Catalog::priced(&[("paid", priced("0.001", "0.002"))]);
    catalog.raw.insert("canonical".to_owned(), priced("0", "0"));
    let mut effective = priced("0", "0");
    effective.insert("key".to_owned(), Scalar::Text("canonical".to_owned()));
    catalog.effective.insert("alias".to_owned(), effective);
    let usage = zero_usage();
    let request = cost_request(&events, &usage, "openai", "paid");
    let total = stream_cost(&request, &catalog, &database(), None::<&mut Breakdown>)
        .expect("the resolved key declares the free entry");
    assert_eq!(total, Decimal::ZERO);
    assert_eq!(catalog.calls(), ["alias"]);
}

#[test]
fn a_null_rate_field_does_not_declare_pricing() {
    let events = [session_event("session.created", Some("null-rate"))];
    let mut catalog = Catalog::priced(&[("paid", priced("0.001", "0.002"))]);
    catalog.raw.insert(
        "null-rate".to_owned(),
        record(&[("input_cost_per_token", Scalar::Null)]),
    );
    catalog
        .effective
        .insert("null-rate".to_owned(), priced("0", "0"));
    let usage = token_usage(100, 10);
    let request = cost_request(&events, &usage, "openai", "paid");
    let total = stream_cost(&request, &catalog, &database(), None::<&mut Breakdown>)
        .expect("the priced candidate wins");
    assert_eq!(total, Decimal::new(12, 2));
    assert_eq!(catalog.calls(), ["null-rate", "paid"]);
}

#[test]
fn cached_prompt_details_reach_generic_pricing() {
    let events = [session_event("session.created", Some("cache-model"))];
    let entry = record(&[
        ("input_cost_per_token", scalar_number("0.001")),
        ("cache_creation_input_token_cost", scalar_number("0.002")),
        ("output_cost_per_token", scalar_number("0.001")),
    ]);
    let catalog = Catalog::priced(&[("cache-model", entry)]);
    let usage = cache_usage(100, 50, 50);
    let request = cost_request(&events, &usage, "openai", "cache-model");
    let total = stream_cost(&request, &catalog, &database(), None::<&mut Breakdown>)
        .expect("the cache rates price the prompt");
    assert_eq!(total, Decimal::new(15, 2));
}
