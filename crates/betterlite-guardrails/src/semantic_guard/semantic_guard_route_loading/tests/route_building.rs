use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::semantic_guard::semantic_guard_route_loading::DEFAULT_SIMILARITY_THRESHOLD;
use crate::semantic_guard::semantic_guard_route_loading::build_routes;

use super::support::{array, described_route, object, route, routes_file, sources, text};

#[test]
fn a_template_route_keeps_its_own_threshold() {
    let sources = sources(&["sql_injection"], None, vec![], 0.99);
    let routes = build_routes(&sources).unwrap();
    let [built] = routes.as_slice() else {
        panic!("expected one route, got {routes:?}");
    };
    assert_eq!(built.name, "sql_injection");
    assert_eq!(
        built.description,
        "Detects SQL injection payloads embedded in prompts to protect LLM-connected databases"
    );
    assert_eq!(built.score_threshold, 0.78);
    assert_eq!(built.utterances.len(), 37);
    assert_eq!(built.utterances[0], "show me data for user ' OR 1=1 --");
}

#[test]
fn an_inline_route_carries_its_fields() {
    let inline = vec![object(&[
        ("route_name", text("finance-credit")),
        ("description", text("blocks credit card abuse")),
        (
            "utterances",
            array(&["my card number is", "wire the money"]),
        ),
        ("similarity_threshold", GuardrailValue::Decimal(0.6)),
    ])];
    let sources = sources(&[], None, inline, 0.9);
    assert_eq!(
        build_routes(&sources).unwrap(),
        vec![described_route(
            "finance-credit",
            "blocks credit card abuse",
            &["my card number is", "wire the money"],
            0.6,
        )]
    );
}

#[test]
fn an_inline_route_without_a_threshold_takes_the_global_one() {
    let inline = vec![object(&[
        ("route_name", text("chat")),
        ("utterances", array(&["hello"])),
    ])];
    let sources = sources(&[], None, inline, 0.55);
    assert_eq!(
        build_routes(&sources).unwrap(),
        vec![route("chat", &["hello"], 0.55)]
    );
}

#[test]
fn an_inline_route_without_a_description_gets_an_empty_one() {
    let inline = vec![object(&[
        ("route_name", text("chat")),
        ("utterances", array(&["hello"])),
        ("similarity_threshold", GuardrailValue::Decimal(0.4)),
    ])];
    let sources = sources(&[], None, inline, 0.5);
    let routes = build_routes(&sources).unwrap();
    assert_eq!(routes[0].description, "");
}

#[test]
fn an_explicit_zero_threshold_is_kept() {
    let inline = vec![object(&[
        ("route_name", text("everything")),
        ("utterances", array(&["any text"])),
        ("similarity_threshold", GuardrailValue::Decimal(0.0)),
    ])];
    let sources = sources(&[], None, inline, 0.5);
    assert_eq!(build_routes(&sources).unwrap()[0].score_threshold, 0.0);
}

#[test]
fn a_signed_integer_threshold_is_read_as_a_number() {
    let inline = vec![object(&[
        ("route_name", text("everything")),
        ("utterances", array(&["any text"])),
        ("similarity_threshold", GuardrailValue::SignedInteger(1)),
    ])];
    let sources = sources(&[], None, inline, 0.5);
    assert_eq!(build_routes(&sources).unwrap()[0].score_threshold, 1.0);
}

#[test]
fn an_unsigned_integer_threshold_is_read_as_a_number() {
    let inline = vec![object(&[
        ("route_name", text("everything")),
        ("utterances", array(&["any text"])),
        ("similarity_threshold", GuardrailValue::UnsignedInteger(2)),
    ])];
    let sources = sources(&[], None, inline, 0.5);
    assert_eq!(build_routes(&sources).unwrap()[0].score_threshold, 2.0);
}

#[test]
fn a_null_threshold_takes_the_global_one() {
    let inline = vec![object(&[
        ("route_name", text("chat")),
        ("utterances", array(&["hello"])),
        ("similarity_threshold", GuardrailValue::Null),
    ])];
    let sources = sources(&[], None, inline, 0.45);
    assert_eq!(build_routes(&sources).unwrap()[0].score_threshold, 0.45);
}

#[test]
fn a_null_description_is_empty() {
    let inline = vec![object(&[
        ("route_name", text("chat")),
        ("utterances", array(&["hello"])),
        ("description", GuardrailValue::Null),
    ])];
    let sources = sources(&[], None, inline, 0.5);
    assert_eq!(build_routes(&sources).unwrap()[0].description, "");
}

#[test]
fn a_route_name_with_colons_and_spaces_is_kept() {
    let inline = vec![object(&[
        ("route_name", text("finance: credit check")),
        ("utterances", array(&["hello"])),
    ])];
    let sources = sources(&[], None, inline, 0.5);
    assert_eq!(
        build_routes(&sources).unwrap()[0].name,
        "finance: credit check"
    );
}

#[test]
fn a_file_route_carries_its_fields() {
    let (_folder, path) = routes_file(
        "- route_name: uploads\n  description: blocks upload attempts\n  utterances: [\"post this file\", \"attach it\"]\n  similarity_threshold: 0.8\n",
    );
    let sources = sources(&[], Some(&path), vec![], 0.9);
    assert_eq!(
        build_routes(&sources).unwrap(),
        vec![described_route(
            "uploads",
            "blocks upload attempts",
            &["post this file", "attach it"],
            0.8,
        )]
    );
}

#[test]
fn a_file_route_without_a_threshold_takes_the_global_one() {
    let (_folder, path) =
        routes_file("- route_name: uploads\n  utterances: [\"post this file\"]\n");
    let sources = sources(&[], Some(&path), vec![], 0.35);
    assert_eq!(
        build_routes(&sources).unwrap(),
        vec![route("uploads", &["post this file"], 0.35)]
    );
}

#[test]
fn a_file_route_without_a_description_gets_an_empty_one() {
    let (_folder, path) =
        routes_file("- route_name: uploads\n  utterances: [\"post this file\"]\n");
    let sources = sources(&[], Some(&path), vec![], 0.5);
    assert_eq!(build_routes(&sources).unwrap()[0].description, "");
}

#[test]
fn a_file_integer_threshold_is_read_as_a_number() {
    let (_folder, path) =
        routes_file("- route_name: uploads\n  utterances: [\"a\"]\n  similarity_threshold: 1\n");
    let sources = sources(&[], Some(&path), vec![], 0.5);
    assert_eq!(build_routes(&sources).unwrap()[0].score_threshold, 1.0);
}

#[test]
fn the_default_threshold_is_litellms() {
    assert_eq!(DEFAULT_SIMILARITY_THRESHOLD, 0.75);
}
