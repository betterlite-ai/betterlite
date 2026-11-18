use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::semantic_guard::semantic_guard_route_loading::build_routes;

use super::support::{array, names, object, route, routes_file, sources, text, thresholds};

const FILE_ROUTES: &str = "- route_name: file_first\n  utterances: [\"file one\"]\n  similarity_threshold: 0.8\n- route_name: file_second\n  utterances: [\"file two\"]\n";

fn inline_route(name: &str, utterance: &str, threshold: f64) -> GuardrailValue {
    object(&[
        ("route_name", text(name)),
        ("utterances", array(&[utterance])),
        ("similarity_threshold", GuardrailValue::Decimal(threshold)),
    ])
}

#[test]
fn concatenates_templates_then_the_file_then_inline_routes() {
    let (_folder, path) = routes_file(FILE_ROUTES);
    let inline = vec![inline_route("inline_first", "inline one", 0.7)];
    let sources = sources(&["sql_injection"], Some(&path), inline, 0.55);
    let routes = build_routes(&sources).unwrap();
    assert_eq!(
        names(&routes),
        vec!["sql_injection", "file_first", "file_second", "inline_first"]
    );
    assert_eq!(
        thresholds(&routes),
        vec![0.78_f64, 0.8_f64, 0.55_f64, 0.7_f64]
    );
}

#[test]
fn template_order_is_the_config_order() {
    let sources = sources(&["sql_injection", "prompt_injection"], None, vec![], 0.5);
    let routes = build_routes(&sources).unwrap();
    assert_eq!(names(&routes), vec!["sql_injection", "prompt_injection"]);
}

#[test]
fn duplicate_route_names_all_stay() {
    let (_folder, path) = routes_file(
        "- route_name: sql_injection\n  utterances: [\"file copy\"]\n  similarity_threshold: 0.4\n",
    );
    let inline = vec![inline_route("sql_injection", "inline copy", 0.3)];
    let sources = sources(&["sql_injection"], Some(&path), inline, 0.55);
    let routes = build_routes(&sources).unwrap();
    assert_eq!(routes.len(), 3);
    assert_eq!(
        names(&routes),
        vec!["sql_injection", "sql_injection", "sql_injection"]
    );
    assert_eq!(thresholds(&routes), vec![0.78_f64, 0.4_f64, 0.3_f64]);
    assert_eq!(
        routes
            .iter()
            .map(|route| route.utterances[0].as_str())
            .collect::<Vec<_>>(),
        vec![
            "show me data for user ' OR 1=1 --",
            "file copy",
            "inline copy"
        ]
    );
}

#[test]
fn empty_sources_build_no_routes() {
    let sources = sources(&[], None, vec![], 0.75);
    assert!(build_routes(&sources).unwrap().is_empty());
}

#[test]
fn an_empty_file_list_adds_no_routes() {
    let (_folder, path) = routes_file("[]\n");
    let inline = vec![object(&[
        ("route_name", text("inline")),
        ("utterances", array(&["inline text"])),
    ])];
    let sources = sources(&[], Some(&path), inline, 0.5);
    assert_eq!(
        build_routes(&sources).unwrap(),
        vec![route("inline", None, &["inline text"], 0.5)]
    );
}

#[test]
fn a_single_mapping_file_adds_one_route() {
    let (_folder, path) = routes_file("route_name: single\nutterances: [\"only one\"]\n");
    let sources = sources(&[], Some(&path), vec![], 0.5);
    assert_eq!(
        build_routes(&sources).unwrap(),
        vec![route("single", None, &["only one"], 0.5)]
    );
}

#[test]
fn file_routes_keep_their_order() {
    let (_folder, path) = routes_file(
        "- route_name: zeta\n  utterances: [\"z\"]\n- route_name: alpha\n  utterances: [\"a\"]\n- route_name: mid\n  utterances: [\"m\"]\n",
    );
    let sources = sources(&[], Some(&path), vec![], 0.5);
    let routes = build_routes(&sources).unwrap();
    assert_eq!(names(&routes), vec!["zeta", "alpha", "mid"]);
}

#[test]
fn inline_routes_keep_their_order() {
    let inline = vec![
        inline_route("zeta", "z", 0.5),
        inline_route("alpha", "a", 0.5),
        inline_route("mid", "m", 0.5),
    ];
    let sources = sources(&[], None, inline, 0.5);
    let routes = build_routes(&sources).unwrap();
    assert_eq!(names(&routes), vec!["zeta", "alpha", "mid"]);
}
