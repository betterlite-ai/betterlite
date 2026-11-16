#![cfg(test)]

use betterlite_core::api::media_relay_logging::error::MediaRelayLoggingError;
use betterlite_core::api::media_relay_logging::fal::{
    FalPassthroughRequest, fal_ai_passthrough_outcome,
};
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use rstest::rstest;

use super::fal::fal_ai_passthrough_handler;
use super::support::{
    Registry, json, kwargs, logged_model, logging_object, lookup, object, registry, spend,
    spend_cost, spend_row,
};

const CATALOG: &[(&str, &str)] = &[(
    "fal_ai/fal-ai/flux/dev",
    r#"{"litellm_provider": "fal_ai", "output_cost_per_image": 0.025}"#,
)];

fn run(
    registry: &Registry,
    logging_obj: &StreamLoggingObject,
    url_route: &str,
    body: &Json,
    request_body: &indexmap::IndexMap<String, Json>,
) -> HandlerOutcome {
    let caller = kwargs(&[]);
    let request = FalPassthroughRequest {
        url_route,
        response_body: body,
        request_body,
        kwargs: &caller,
        lookup: &registry.lookup,
        queue_base: None,
    };
    fal_ai_passthrough_handler(logging_obj, &request).expect("the handler runs")
}

#[test]
fn fal_ai_passthrough_bills_the_submission() {
    let registry = registry(CATALOG);
    let logging_obj = logging_object();
    let body = json(
        r#"{"images": [{"url": "https://fal.example/a.png"}, {"url": "https://fal.example/b.png"}],
            "status": "OK", "request_id": "r-1"}"#,
    );
    let request_body = object(&[("num_images", json("2"))]);
    let caller = kwargs(&[("custom", json("1"))]);
    let outcome = fal_ai_passthrough_handler(
        &logging_obj,
        &FalPassthroughRequest {
            url_route: "https://queue.fal.run/fal-ai/flux/dev",
            response_body: &body,
            request_body: &request_body,
            kwargs: &caller,
            lookup: &registry.lookup,
            queue_base: None,
        },
    )
    .expect("the handler runs");

    assert_eq!(logged_model(&logging_obj), "fal-ai/flux/dev");
    assert_eq!(
        spend(&logging_obj),
        spend_row("fal-ai/flux/dev", "fal_ai", json("0.05"))
    );
    let mut expected_kwargs = spend_row("fal-ai/flux/dev", "fal_ai", json("0.05"));
    expected_kwargs.insert("custom".to_owned(), json("1"));
    assert_eq!(outcome.kwargs.entries, expected_kwargs);
    assert_eq!(
        outcome.result.expect("a result").into_json(),
        json(
            r#"{"data": [{"url": "https://fal.example/a.png"}, {"url": "https://fal.example/b.png"}],
                "_hidden_params": {"response_cost": 0.05}}"#
        )
    );
    assert_eq!(caller.entries.len(), 1);
}

#[test]
fn fal_ai_passthrough_strips_the_queue_base_path() {
    let registry = registry(CATALOG);
    let logging_obj = logging_object();
    let body = json(r#"{"images": [{"url": "https://fal.example/a.png"}]}"#);
    let request_body = object(&[]);

    let caller = kwargs(&[]);
    let outcome = fal_ai_passthrough_handler(
        &logging_obj,
        &FalPassthroughRequest {
            url_route: "https://queue.example/api/fal-ai/flux/dev",
            response_body: &body,
            request_body: &request_body,
            kwargs: &caller,
            lookup: &registry.lookup,
            queue_base: Some("https://queue.example/api/"),
        },
    )
    .expect("the handler runs");

    assert_eq!(logged_model(&logging_obj), "fal-ai/flux/dev");
    assert_eq!(spend_cost(&logging_obj), json("0.025"));
    assert_eq!(
        outcome.result.expect("a result").into_json(),
        json(
            r#"{"data": [{"url": "https://fal.example/a.png"}],
                "_hidden_params": {"response_cost": 0.025}}"#
        )
    );
}

#[test]
fn fal_ai_passthrough_keeps_a_route_outside_the_queue_base_path() {
    let registry = registry(CATALOG);
    let logging_obj = logging_object();
    let body = json(r#"{"images": []}"#);
    let request_body = object(&[]);
    let caller = kwargs(&[]);
    let request = FalPassthroughRequest {
        url_route: "https://queue.fal.run/fal-ai/flux/dev",
        response_body: &body,
        request_body: &request_body,
        kwargs: &caller,
        lookup: &registry.lookup,
        queue_base: Some("https://queue.example/api"),
    };

    fal_ai_passthrough_handler(&logging_obj, &request).expect("the handler runs");

    assert_eq!(logged_model(&logging_obj), "fal-ai/flux/dev");
    assert_eq!(spend_cost(&logging_obj), json("0.025"));
}

#[rstest]
#[case::status("https://queue.fal.run/fal-ai/flux/dev/requests/r-1/status")]
#[case::result("https://queue.fal.run/fal-ai/flux/dev/requests/r-1")]
fn fal_ai_queue_status_routes_skip_the_cost(#[case] url_route: &str) {
    let registry = registry(CATALOG);
    let logging_obj = logging_object();
    let body = json(r#"{"status": "COMPLETED", "request_id": "r-1"}"#);
    let request_body = object(&[("num_images", json("2"))]);

    let outcome = run(&registry, &logging_obj, url_route, &body, &request_body);

    assert_eq!(logged_model(&logging_obj), "fal-ai/flux/dev");
    assert_eq!(
        spend(&logging_obj),
        spend_row("fal-ai/flux/dev", "fal_ai", Json::Null)
    );
    assert_eq!(
        outcome.result.expect("a result").into_json(),
        json(r#"{"data": [], "_hidden_params": {"response_cost": null}}"#)
    );
}

#[test]
fn fal_ai_passthrough_missing_registry_entry_keeps_the_spend_row() {
    let registry = registry(&[]);
    let logging_obj = logging_object();
    let body = json(r#"{"images": [{"url": "https://fal.example/a.png"}]}"#);
    let request_body = object(&[]);

    let outcome = run(
        &registry,
        &logging_obj,
        "https://queue.fal.run/fal-ai/unpriced",
        &body,
        &request_body,
    );

    assert_eq!(logged_model(&logging_obj), "fal-ai/unpriced");
    assert_eq!(spend_cost(&logging_obj), Json::Null);
    assert_eq!(
        outcome.result.expect("a result").into_json(),
        json(
            r#"{"data": [{"url": "https://fal.example/a.png"}],
                "_hidden_params": {"response_cost": null}}"#
        )
    );
}

#[test]
fn fal_ai_passthrough_bills_every_requested_image() {
    let registry = registry(CATALOG);
    let logging_obj = logging_object();
    let body = json(r#"{"images": []}"#);
    let request_body = object(&[("num_images", json("3"))]);

    run(
        &registry,
        &logging_obj,
        "https://queue.fal.run/fal-ai/flux/dev",
        &body,
        &request_body,
    );

    assert_eq!(spend_cost(&logging_obj), json("0.075"));
}

#[test]
fn fal_ai_passthrough_reads_a_resolution_price_first() {
    let registry = registry(&[(
        "fal_ai/fal-ai/flux/dev",
        r#"{"litellm_provider": "fal_ai", "output_cost_per_image": 0.025, "output_cost_per_image_1K": 0.05}"#,
    )]);
    let logging_obj = logging_object();
    let body = json(r#"{"images": []}"#);
    let request_body = object(&[("resolution", json(r#""1K""#))]);

    run(
        &registry,
        &logging_obj,
        "https://queue.fal.run/fal-ai/flux/dev",
        &body,
        &request_body,
    );

    assert_eq!(spend_cost(&logging_obj), json("0.05"));
}

#[test]
fn fal_ai_passthrough_collects_top_level_urls_only() {
    let registry = registry(CATALOG);
    let logging_obj = logging_object();
    let body = json(
        r#"{
            "image": {"url": "https://fal.example/a.png"},
            "images": [{"url": "https://fal.example/b.png"}, {"url": 5}, "not an object"],
            "nested": {"inner": {"url": "https://fal.example/deep.png"}},
            "nested_list": [[{"url": "https://fal.example/deeper.png"}]],
            "text": "https://fal.example/not-a-field.png",
            "request_id": "r-1"
        }"#,
    );
    let request_body = object(&[]);

    let outcome = run(
        &registry,
        &logging_obj,
        "https://queue.fal.run/fal-ai/flux/dev",
        &body,
        &request_body,
    );

    assert_eq!(
        outcome.result.expect("a result").into_json(),
        json(
            r#"{"data": [{"url": "https://fal.example/a.png"}, {"url": "https://fal.example/b.png"}],
                "_hidden_params": {"response_cost": 0.025}}"#
        )
    );
}

#[test]
fn fal_ai_passthrough_cost_beyond_the_decimal_range_is_a_pricing_error() {
    let registry = Registry {
        catalog: registry(&[]).catalog,
        lookup: lookup(&[(
            "fal_ai/fal-ai/huge",
            r#"{"litellm_provider": "fal_ai", "output_cost_per_image": 7.9e28}"#,
        )]),
    };
    let body = json(r#"{"images": []}"#);
    let request_body = object(&[("num_images", json("2"))]);
    let caller = kwargs(&[]);
    let request = FalPassthroughRequest {
        url_route: "https://queue.fal.run/fal-ai/huge",
        response_body: &body,
        request_body: &request_body,
        kwargs: &caller,
        lookup: &registry.lookup,
        queue_base: None,
    };

    let error =
        fal_ai_passthrough_outcome(&request).expect_err("the cost leaves the decimal range");

    assert!(matches!(error, MediaRelayLoggingError::FalPricing(_)));
}
