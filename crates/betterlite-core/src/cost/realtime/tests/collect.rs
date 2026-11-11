use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::{UsageNumber, usage_sum};
use rust_decimal::Decimal;

use crate::cost::realtime::error::RealtimeCostError;
use crate::cost::realtime::realtime_stream;
use crate::cost::realtime::responses_ws;
use crate::cost::realtime::{Event, RealtimeBilling};

use super::support::{event, number, object, response_usage, text, zero_usage};

#[test]
fn realtime_collection_reads_only_response_done() {
    let events = [
        event(&[("type", text("session.created"))]),
        event(&[
            ("type", text("response.completed")),
            ("response", object(&[("usage", response_usage("4", "1"))])),
        ]),
        event(&[
            ("type", text("response.done")),
            ("response", object(&[("usage", response_usage("10", "4"))])),
        ]),
    ];
    let usages = realtime_stream::collect_usage(&events).expect("the events are readable");
    assert_eq!(usages.len(), 1);
    assert_eq!(usages[0].prompt_tokens, 10);
    assert_eq!(usages[0].completion_tokens, 4);
    assert_eq!(usages[0].total_tokens, 14);
}

#[test]
fn a_missing_realtime_type_is_a_typed_failure() {
    let events = [event(&[("response", object(&[]))])];
    let error = realtime_stream::collect_usage(&events).expect_err("type is required");
    assert!(matches!(
        error,
        RealtimeCostError::MissingField { field: "type" }
    ));
}

#[test]
fn a_missing_realtime_response_is_a_typed_failure() {
    let events = [event(&[("type", text("response.done"))])];
    let error = realtime_stream::collect_usage(&events).expect_err("response is required");
    assert!(matches!(
        error,
        RealtimeCostError::MissingField { field: "response" }
    ));
}

#[test]
fn a_non_object_realtime_response_is_a_typed_failure() {
    let events = [event(&[
        ("type", text("response.done")),
        ("response", text("x")),
    ])];
    let error = realtime_stream::collect_usage(&events).expect_err("response must be an object");
    assert!(matches!(
        error,
        RealtimeCostError::Shape {
            field: "response",
            ..
        }
    ));
}

#[test]
fn absent_and_null_realtime_usage_both_bill_zero() {
    let absent = event(&[("type", text("response.done")), ("response", object(&[]))]);
    let null = event(&[
        ("type", text("response.done")),
        ("response", object(&[("usage", Json::Null)])),
    ]);
    let usages = realtime_stream::collect_usage(&[absent, null]).expect("both bill zero");
    assert_eq!(usages.len(), 2);
    assert_eq!(usages[0], zero_usage());
    assert_eq!(usages[1], zero_usage());
}

#[test]
fn realtime_usage_keeps_prompt_and_completion_details() {
    let events = [event(&[
        ("type", text("response.done")),
        (
            "response",
            object(&[(
                "usage",
                object(&[
                    ("input_tokens", number("10")),
                    ("output_tokens", number("6")),
                    (
                        "input_token_details",
                        object(&[("text_tokens", number("8")), ("audio_tokens", number("2"))]),
                    ),
                    (
                        "output_token_details",
                        object(&[
                            ("text_tokens", number("4")),
                            ("reasoning_tokens", number("2")),
                        ]),
                    ),
                ]),
            )]),
        ),
    ])];
    let usages = realtime_stream::collect_usage(&events).expect("the usage is readable");
    let prompt = usages[0]
        .prompt_tokens_details
        .as_ref()
        .expect("prompt details are kept");
    assert_eq!(prompt.text_tokens, Some(8));
    assert_eq!(prompt.audio_tokens, Some(2));
    let completion = usages[0]
        .completion_tokens_details
        .as_ref()
        .expect("completion details are kept");
    assert_eq!(completion.text_tokens, Some(4));
    assert_eq!(completion.reasoning_tokens, Some(2));
}

#[test]
fn combined_realtime_usage_sums_each_response_done_event() {
    let events = [
        event(&[
            ("type", text("response.done")),
            ("response", object(&[("usage", response_usage("10", "20"))])),
        ]),
        event(&[
            ("type", text("response.done")),
            ("response", object(&[("usage", response_usage("5", "15"))])),
        ]),
    ];
    let usages = realtime_stream::collect_usage(&events).expect("the events are readable");
    let combined = usage_sum(&usages).expect("the usages add");
    assert_eq!(combined.prompt_tokens, 15);
    assert_eq!(combined.completion_tokens, 35);
    assert_eq!(combined.total_tokens, 50);
}

#[test]
fn responses_ws_collects_completed_and_incomplete_only() {
    let events = [
        event(&[
            ("type", text("response.done")),
            ("response", object(&[("usage", response_usage("9", "9"))])),
        ]),
        event(&[
            ("type", text("response.completed")),
            ("response", object(&[("usage", response_usage("3", "1"))])),
        ]),
        event(&[
            ("type", text("response.incomplete")),
            ("response", object(&[("usage", response_usage("2", "0"))])),
        ]),
        event(&[("type", text("response.failed")), ("response", object(&[]))]),
    ];
    let usages = responses_ws::collect_usage(&events).expect("the events are readable");
    assert_eq!(usages.len(), 2);
    assert_eq!(usages[0].prompt_tokens, 3);
    assert_eq!(usages[1].prompt_tokens, 2);
    let ws_usages = responses_ws::collect_usage(&events).expect("the events are readable");
    let combined = usage_sum(&ws_usages).expect("the usages add");
    assert_eq!(combined.total_tokens, 6);
}

#[test]
fn empty_usage_counts_and_null_usage_does_not() {
    let events = [
        event(&[
            ("type", text("response.completed")),
            ("response", object(&[("usage", object(&[]))])),
        ]),
        event(&[
            ("type", text("response.completed")),
            ("response", object(&[("usage", Json::Null)])),
        ]),
        event(&[
            ("type", text("response.incomplete")),
            ("response", object(&[])),
        ]),
    ];
    let usages = responses_ws::collect_usage(&events).expect("the events are readable");
    assert_eq!(usages.len(), 1);
    assert_eq!(usages[0], zero_usage());
}

#[test]
fn an_unrelated_malformed_event_still_validates() {
    let events = [
        event(&[
            ("type", text("response.completed")),
            ("response", object(&[("usage", response_usage("2", "1"))])),
        ]),
        event(&[
            ("type", text("rate_limits.updated")),
            ("response", object(&[("usage", number("5"))])),
        ]),
    ];
    let error = responses_ws::collect_usage(&events).expect_err("every event validates");
    assert!(matches!(
        error,
        RealtimeCostError::Shape { field: "usage", .. }
    ));
}

#[test]
fn a_malformed_service_tier_is_a_typed_failure() {
    let events = [event(&[
        ("type", text("response.completed")),
        (
            "response",
            object(&[("service_tier", number("1")), ("usage", object(&[]))]),
        ),
    ])];
    let error = responses_ws::collect_usage(&events).expect_err("the tier must be a string");
    assert!(matches!(
        error,
        RealtimeCostError::Shape {
            field: "service_tier",
            ..
        }
    ));
}

fn tiered_event(event_type: &str, tier: Option<&str>, event_id: &str, input: &str) -> Event {
    let mut response = event(&[("usage", response_usage(input, "0"))]);
    if let Some(found) = tier {
        response.insert("service_tier".to_owned(), text(found));
    }
    event(&[
        ("type", text(event_type)),
        ("response", Json::Object(response)),
        ("event_id", text(event_id)),
    ])
}

#[test]
fn partitions_keep_first_seen_tiers_and_event_order() {
    let first = tiered_event("response.completed", Some("priority"), "first", "1");
    let second = tiered_event("response.completed", Some(""), "second", "2");
    let third = tiered_event("response.completed", None, "third", "3");
    let fourth = event(&[
        ("type", text("response.incomplete")),
        (
            "response",
            object(&[
                ("service_tier", Json::Null),
                ("usage", response_usage("4", "0")),
            ]),
        ),
        ("event_id", text("fourth")),
    ]);
    let partition = responses_ws::partition_by_service_tier(&[first, second, third, fourth])
        .expect("the events are readable");
    let tiers: Vec<Option<&str>> = partition.keys().map(Option::as_deref).collect();
    assert_eq!(tiers, [Some("priority"), Some(""), None]);
    let priority = partition
        .get(&Some("priority".to_owned()))
        .expect("priority");
    let empty = partition.get(&Some(String::new())).expect("empty tier");
    let missing = partition.get(&None).expect("missing tier");
    assert_eq!(priority.len(), 1);
    assert_eq!(empty.len(), 1);
    assert_eq!(missing.len(), 2);
    assert_eq!(priority[0].get("event_id"), Some(&text("first")));
    assert_eq!(missing[0].get("event_id"), Some(&text("third")));
    assert_eq!(missing[1].get("event_id"), Some(&text("fourth")));
}

#[test]
fn nonbillable_events_stay_out_of_every_group() {
    let events = [
        event(&[
            ("type", text("response.completed")),
            (
                "response",
                object(&[("service_tier", text("default")), ("usage", object(&[]))]),
            ),
        ]),
        event(&[
            ("type", text("response.created")),
            ("response", object(&[("service_tier", text("priority"))])),
        ]),
    ];
    let partition =
        responses_ws::partition_by_service_tier(&events).expect("the events are readable");
    assert_eq!(partition.len(), 1);
    let group = partition.get(&Some("default".to_owned())).expect("default");
    assert_eq!(group.len(), 1);
}

#[test]
fn one_tier_builds_no_split() {
    let record = RealtimeBilling {
        results: vec![event(&[
            ("type", text("response.completed")),
            (
                "response",
                object(&[
                    ("service_tier", text("default")),
                    ("usage", response_usage("2", "1")),
                ]),
            ),
        ])],
        usage: zero_usage(),
        service_tier: None,
    };
    assert_eq!(
        responses_ws::split_by_service_tier(&record).expect("the record is readable"),
        None
    );
}

#[test]
fn no_billable_event_builds_no_split() {
    let record = RealtimeBilling {
        results: vec![event(&[("type", text("response.created"))])],
        usage: zero_usage(),
        service_tier: None,
    };
    assert_eq!(
        responses_ws::split_by_service_tier(&record).expect("the record is readable"),
        None
    );
}

#[test]
fn several_tiers_split_into_independently_aggregated_records() {
    let first = event(&[
        ("type", text("response.completed")),
        (
            "response",
            object(&[
                ("service_tier", text("default")),
                ("usage", response_usage("100", "40")),
            ]),
        ),
    ]);
    let unrelated = event(&[("type", text("rate_limits.updated"))]);
    let second = event(&[
        ("type", text("response.completed")),
        (
            "response",
            object(&[
                ("service_tier", text("priority")),
                ("usage", response_usage("60", "10")),
            ]),
        ),
    ]);
    let record = RealtimeBilling {
        results: vec![first, unrelated, second],
        usage: zero_usage(),
        service_tier: None,
    };
    let parts = responses_ws::split_by_service_tier(&record)
        .expect("the record is readable")
        .expect("two tiers build a split");
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].service_tier, Some("default".to_owned()));
    assert_eq!(parts[0].usage.prompt_tokens, 100);
    assert_eq!(parts[0].usage.completion_tokens, 40);
    assert_eq!(parts[0].results.len(), 1);
    assert_eq!(parts[1].service_tier, Some("priority".to_owned()));
    assert_eq!(parts[1].usage.prompt_tokens, 60);
    assert_eq!(parts[1].usage.completion_tokens, 10);
    assert_eq!(parts[1].results.len(), 1);
}

fn cost_event(event_type: &str, cost: &str) -> Event {
    event(&[
        ("type", text(event_type)),
        (
            "response",
            object(&[(
                "usage",
                object(&[
                    ("input_tokens", number("2")),
                    ("output_tokens", number("3")),
                    ("total_tokens", number("5")),
                    ("cost", number(cost)),
                ]),
            )]),
        ),
    ])
}

#[test]
fn provider_cost_is_summed_across_collected_events() {
    let realtime_events = [
        cost_event("response.done", "0.125"),
        cost_event("response.done", "0.125"),
    ];
    let realtime_usages =
        realtime_stream::collect_usage(&realtime_events).expect("cost is a usage extra");
    assert_eq!(
        realtime_usages[0].numeric_extras.get("cost"),
        Some(&UsageNumber::Decimal(Decimal::new(125, 3)))
    );
    let realtime_combined = usage_sum(&realtime_usages).expect("the costs add");
    assert_eq!(
        realtime_combined.numeric_extras.get("cost"),
        Some(&UsageNumber::Decimal(Decimal::new(25, 2)))
    );

    let ws_events = [
        cost_event("response.completed", "0.125"),
        cost_event("response.completed", "0.125"),
    ];
    let ws_usages = responses_ws::collect_usage(&ws_events).expect("cost is a usage extra");
    let ws_combined = usage_sum(&ws_usages).expect("the costs add");
    assert_eq!(
        ws_combined.numeric_extras.get("cost"),
        Some(&UsageNumber::Decimal(Decimal::new(25, 2)))
    );
}
