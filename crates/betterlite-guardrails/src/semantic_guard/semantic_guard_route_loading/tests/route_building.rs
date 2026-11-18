use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::semantic_guard::semantic_guard_route_loading::DEFAULT_SIMILARITY_THRESHOLD;
use crate::semantic_guard::semantic_guard_route_loading::build_routes;
use crate::semantic_guard::semantic_guard_route_loading::custom_routes::quote_pyyaml_text_scalars;
use crate::semantic_guard::semantic_guard_route_loading::default_similarity_threshold;

use super::support::{array, environment, object, route, routes_file, sources, text};

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
    assert_eq!(built.score_threshold.to_bits(), 0.78_f64.to_bits());
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
        vec![route(
            "finance-credit",
            Some("blocks credit card abuse"),
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
        vec![route("chat", None, &["hello"], 0.55)]
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
    assert_eq!(
        build_routes(&sources).unwrap()[0].score_threshold.to_bits(),
        0.0_f64.to_bits()
    );
}

#[test]
fn a_signed_integer_threshold_is_read_as_a_number() {
    let inline = vec![object(&[
        ("route_name", text("everything")),
        ("utterances", array(&["any text"])),
        ("similarity_threshold", GuardrailValue::SignedInteger(1)),
    ])];
    let sources = sources(&[], None, inline, 0.5);
    assert_eq!(
        build_routes(&sources).unwrap()[0].score_threshold.to_bits(),
        1.0_f64.to_bits()
    );
}

#[test]
fn an_unsigned_integer_threshold_is_read_as_a_number() {
    let inline = vec![object(&[
        ("route_name", text("everything")),
        ("utterances", array(&["any text"])),
        ("similarity_threshold", GuardrailValue::UnsignedInteger(2)),
    ])];
    let sources = sources(&[], None, inline, 0.5);
    assert_eq!(
        build_routes(&sources).unwrap()[0].score_threshold.to_bits(),
        2.0_f64.to_bits()
    );
}

#[test]
fn a_null_threshold_takes_the_global_one() {
    let inline = vec![object(&[
        ("route_name", text("chat")),
        ("utterances", array(&["hello"])),
        ("similarity_threshold", GuardrailValue::Null),
    ])];
    let sources = sources(&[], None, inline, 0.45);
    assert_eq!(
        build_routes(&sources).unwrap()[0].score_threshold.to_bits(),
        0.45_f64.to_bits()
    );
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
        vec![route(
            "uploads",
            Some("blocks upload attempts"),
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
        vec![route("uploads", None, &["post this file"], 0.35)]
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
    assert_eq!(
        build_routes(&sources).unwrap()[0].score_threshold.to_bits(),
        1.0_f64.to_bits()
    );
}

#[test]
fn the_default_threshold_is_litellms() {
    assert_eq!(DEFAULT_SIMILARITY_THRESHOLD.to_bits(), 0.75_f64.to_bits());
}

#[test]
fn the_default_threshold_is_read_from_the_environment() {
    assert_eq!(
        default_similarity_threshold(&environment("0.91"))
            .unwrap()
            .to_bits(),
        0.91_f64.to_bits()
    );
}

#[test]
fn an_unset_threshold_variable_keeps_litellms_default() {
    let unset: BTreeMap<String, String> = BTreeMap::new();
    assert_eq!(
        default_similarity_threshold(&unset).unwrap().to_bits(),
        DEFAULT_SIMILARITY_THRESHOLD.to_bits()
    );
}

#[test]
fn an_empty_threshold_variable_keeps_litellms_default() {
    assert_eq!(
        default_similarity_threshold(&environment(""))
            .unwrap()
            .to_bits(),
        DEFAULT_SIMILARITY_THRESHOLD.to_bits()
    );
}

#[test]
fn a_quoted_scalar_utterance_stays_text() {
    let (_folder, path) = routes_file("- route_name: quoted\n  utterances: [\"1\", \"true\"]\n");
    let sources = sources(&[], Some(&path), vec![], 0.5);
    assert_eq!(
        build_routes(&sources).unwrap()[0].utterances,
        vec!["1".to_owned(), "true".to_owned()]
    );
}

#[test]
fn a_file_scalar_pyyaml_reads_as_text_stays_text() {
    let (_folder, path) = routes_file(
        "- route_name: 1e3\n  description: y\n  utterances: [1e3, 1.5e3, 0o10, y, 1e-3, 1.5E3, .5e3]\n  similarity_threshold: 0.8\n  metadata: {source: pyyaml}\n",
    );
    let sources = sources(&[], Some(&path), vec![], 0.5);
    assert_eq!(
        build_routes(&sources).unwrap(),
        vec![route(
            "1e3",
            Some("y"),
            &["1e3", "1.5e3", "0o10", "y", "1e-3", "1.5E3", ".5e3"],
            0.8,
        )]
    );
}

#[test]
fn a_file_utterance_that_overflows_f64_stays_text() {
    let (_folder, path) = routes_file("- route_name: overflow\n  \"utterances\": [1e309, 1e999]\n");
    let sources = sources(&[], Some(&path), vec![], 0.5);
    assert_eq!(
        build_routes(&sources).unwrap()[0].utterances,
        vec!["1e309".to_owned(), "1e999".to_owned()]
    );
}

#[test]
fn a_root_sequence_does_not_quote_non_route_scalars() {
    let source = b"- 1e309\n";
    assert_eq!(quote_pyyaml_text_scalars(source).as_ref(), source);
}

#[test]
fn a_file_case_variant_null_stays_text() {
    let (_folder, path) =
        routes_file("- route_name: nUll\n  description: nULL\n  utterances: [NuLL, nuLL]\n");
    let sources = sources(&[], Some(&path), vec![], 0.5);
    assert_eq!(
        build_routes(&sources).unwrap(),
        vec![route("nUll", Some("nULL"), &["NuLL", "nuLL"], 0.5)]
    );
}

#[test]
fn a_file_merge_key_keeps_merged_route_fields() {
    let (_folder, path) = routes_file(
        "- <<: {description: merged description, similarity_threshold: 0.8}\n  route_name: merged_route\n  utterances: [merged utterance]\n",
    );
    let sources = sources(&[], Some(&path), vec![], 0.5);
    assert_eq!(
        build_routes(&sources).unwrap(),
        vec![route(
            "merged_route",
            Some("merged description"),
            &["merged utterance"],
            0.8
        )]
    );
}

#[test]
fn a_file_merge_alias_keeps_merged_route_fields() {
    let (_folder, path) = routes_file(
        "- &defaults {route_name: base_route, utterances: [base utterance], description: merged description, similarity_threshold: 0.8}\n- <<: *defaults\n  route_name: alias_route\n  utterances: [alias utterance]\n",
    );
    let sources = sources(&[], Some(&path), vec![], 0.5);
    assert_eq!(
        build_routes(&sources).unwrap()[1],
        route(
            "alias_route",
            Some("merged description"),
            &["alias utterance"],
            0.8
        )
    );
}

#[test]
fn a_pyyaml_timestamp_or_sexagesimal_scalar_stays_text() {
    let (_folder, path) = routes_file(
        "- route_name: 2020-01-01\n  description: 1:30\n  utterances:\n    - 2020-01-01\n    - 1:30\n    - 1:30.5\n",
    );
    let sources = sources(&[], Some(&path), vec![], 0.5);
    assert_eq!(
        build_routes(&sources).unwrap(),
        vec![route(
            "2020-01-01",
            Some("1:30"),
            &["2020-01-01", "1:30", "1:30.5"],
            0.5,
        )]
    );
}

#[test]
fn a_pyyaml_underscored_number_stays_text() {
    let (_folder, path) = routes_file("- route_name: 0_1\n  utterances:\n    - 1_\n    - 1__0\n");
    let sources = sources(&[], Some(&path), vec![], 0.5);
    assert_eq!(
        build_routes(&sources).unwrap(),
        vec![route("0_1", None, &["1_", "1__0"], 0.5)]
    );
}

#[test]
fn a_file_octal_threshold_is_read_by_the_yaml_readers_inference() {
    let (_folder, path) = routes_file(
        "- route_name: explicit\n  utterances: [\"a\"]\n  similarity_threshold: 0o10\n- route_name: leading\n  utterances: [\"b\"]\n  similarity_threshold: 0127\n",
    );
    let sources = sources(&[], Some(&path), vec![], 0.5);
    let routes = build_routes(&sources).unwrap();
    assert_eq!(routes[0].score_threshold.to_bits(), 8.0_f64.to_bits());
    assert_eq!(routes[1].score_threshold.to_bits(), 127.0_f64.to_bits());
}

#[test]
fn a_file_uppercase_radix_threshold_is_read_by_the_yaml_readers_inference() {
    let (_folder, path) = routes_file(
        "- route_name: uppercase\n  utterances: [\"a\"]\n  similarity_threshold: 0X1f\n",
    );
    let sources = sources(&[], Some(&path), vec![], 0.5);
    assert_eq!(
        build_routes(&sources).unwrap()[0].score_threshold.to_bits(),
        31.0_f64.to_bits()
    );
}
