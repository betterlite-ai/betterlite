use crate::semantic_guard::semantic_guard_route_loading::build_routes;
use crate::semantic_guard::semantic_guard_route_loading::builder::{
    RouterSetupError, build_router, load_and_build_router,
};
use crate::semantic_guard::semantic_guard_route_loading::error::RouteLoadError;

use super::support::{FakeBuilder, array, block_on, object, routes_file, sources, text};

#[test]
fn the_builder_receives_the_routes_and_the_global_threshold_once() {
    let (_folder, path) = routes_file(
        "- route_name: file_route\n  utterances: [\"file text\"]\n  similarity_threshold: 0.8\n",
    );
    let inline = vec![object(&[
        ("route_name", text("inline_route")),
        ("utterances", array(&["inline text"])),
    ])];
    let sources = sources(&["sql_injection"], Some(&path), inline, 0.55);
    let routes = build_routes(&sources).unwrap();
    let builder = FakeBuilder::passing();
    let names = block_on(build_router(
        &builder,
        routes.clone(),
        sources.global_threshold,
    ))
    .unwrap();
    assert_eq!(
        names,
        vec![
            "sql_injection".to_owned(),
            "file_route".to_owned(),
            "inline_route".to_owned()
        ]
    );
    let calls = builder.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].routes, routes);
    assert_eq!(calls[0].threshold.to_bits(), 0.55_f64.to_bits());
}

#[test]
fn duplicate_names_reach_the_builder_in_load_order() {
    let (_folder, path) =
        routes_file("- route_name: sql_injection\n  utterances: [\"file copy\"]\n");
    let inline = vec![object(&[
        ("route_name", text("sql_injection")),
        ("utterances", array(&["inline copy"])),
    ])];
    let sources = sources(&["sql_injection"], Some(&path), inline, 0.5);
    let routes = build_routes(&sources).unwrap();
    let builder = FakeBuilder::passing();
    let names = block_on(build_router(
        &builder,
        routes.clone(),
        sources.global_threshold,
    ))
    .unwrap();
    assert_eq!(names, vec!["sql_injection".to_owned(); 3]);
    let calls = builder.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].routes, routes);
    assert_eq!(calls[0].routes[1].utterances, vec!["file copy".to_owned()]);
    assert_eq!(
        calls[0].routes[2].utterances,
        vec!["inline copy".to_owned()]
    );
}

#[test]
fn empty_sources_reach_the_builder_once() {
    let sources = sources(&[], None, vec![], 0.4);
    let routes = build_routes(&sources).unwrap();
    let builder = FakeBuilder::passing();
    let names = block_on(build_router(&builder, routes, sources.global_threshold)).unwrap();
    assert!(names.is_empty());
    let calls = builder.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].routes.is_empty());
    assert_eq!(calls[0].threshold.to_bits(), 0.4_f64.to_bits());
}

#[test]
fn a_build_failure_is_reported() {
    let builder = FakeBuilder::failing();
    let error = block_on(build_router(&builder, Vec::new(), 0.5)).unwrap_err();
    assert_eq!(
        error.to_string(),
        "the semantic router could not be built: the fake semantic router build failed"
    );
}

#[test]
fn a_load_failure_never_reaches_the_builder() {
    let sources = sources(&["not_a_template"], None, vec![], 0.5);
    let builder = FakeBuilder::passing();
    let error = block_on(load_and_build_router(&builder, &sources)).unwrap_err();
    assert!(matches!(
        error,
        RouterSetupError::Load(RouteLoadError::UnknownTemplate { .. })
    ));
    assert!(builder.calls.lock().unwrap().is_empty());
}

#[test]
fn the_composed_caller_builds_once_from_the_loaded_routes() {
    let (_folder, path) = routes_file("- route_name: file_route\n  utterances: [\"file text\"]\n");
    let sources = sources(&[], Some(&path), vec![], 0.62);
    let builder = FakeBuilder::passing();
    let names = block_on(load_and_build_router(&builder, &sources)).unwrap();
    assert_eq!(names, vec!["file_route".to_owned()]);
    let calls = builder.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].routes[0].utterances, vec!["file text".to_owned()]);
    assert_eq!(calls[0].threshold.to_bits(), 0.62_f64.to_bits());
}

#[test]
fn a_build_failure_from_the_composed_caller_is_reported() {
    let sources = sources(&[], None, vec![], 0.5);
    let builder = FakeBuilder::failing();
    let error = block_on(load_and_build_router(&builder, &sources)).unwrap_err();
    assert!(matches!(&error, RouterSetupError::Build(_)));
    assert_eq!(
        error.to_string(),
        "the semantic router could not be built: the fake semantic router build failed"
    );
}
