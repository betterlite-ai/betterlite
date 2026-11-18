use crate::semantic_guard::semantic_guard_route_loading::builder::{
    RouterBuildError, build_router,
};
use crate::semantic_guard::semantic_guard_route_loading::error::RouteLoadError;

use super::support::{FakeBuilder, array, object, routes_file, sources, text};

#[tokio::test]
async fn the_builder_receives_the_routes_and_the_global_threshold_once() {
    let (_folder, path) = routes_file(
        "- route_name: file_route\n  utterances: [\"file text\"]\n  similarity_threshold: 0.8\n",
    );
    let inline = vec![object(&[
        ("route_name", text("inline_route")),
        ("utterances", array(&["inline text"])),
    ])];
    let sources = sources(&["sql_injection"], Some(&path), inline, 0.55);
    let builder = FakeBuilder::passing();
    let names = build_router(&builder, &sources).await.unwrap();
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
    assert_eq!(calls[0].names, names);
    assert_eq!(calls[0].threshold, 0.55);
}

#[tokio::test]
async fn duplicate_names_reach_the_builder_in_load_order() {
    let (_folder, path) =
        routes_file("- route_name: sql_injection\n  utterances: [\"file copy\"]\n");
    let inline = vec![object(&[
        ("route_name", text("sql_injection")),
        ("utterances", array(&["inline copy"])),
    ])];
    let sources = sources(&["sql_injection"], Some(&path), inline, 0.5);
    let builder = FakeBuilder::passing();
    let names = build_router(&builder, &sources).await.unwrap();
    assert_eq!(names, vec!["sql_injection".to_owned(); 3]);
}

#[tokio::test]
async fn empty_sources_reach_the_builder_once() {
    let sources = sources(&[], None, vec![], 0.4);
    let builder = FakeBuilder::passing();
    let names = build_router(&builder, &sources).await.unwrap();
    assert!(names.is_empty());
    let calls = builder.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].names.is_empty());
    assert_eq!(calls[0].threshold, 0.4);
}

#[tokio::test]
async fn a_build_failure_is_reported() {
    let sources = sources(&[], None, vec![], 0.5);
    let builder = FakeBuilder::failing();
    let error = build_router(&builder, &sources).await.unwrap_err();
    assert!(matches!(&error, RouterBuildError::Build { .. }));
    assert_eq!(
        error.to_string(),
        "the semantic router could not be built: the fake semantic router build failed"
    );
}

#[tokio::test]
async fn a_load_failure_never_reaches_the_builder() {
    let sources = sources(&["not_a_template"], None, vec![], 0.5);
    let builder = FakeBuilder::passing();
    let error = build_router(&builder, &sources).await.unwrap_err();
    assert!(matches!(
        &error,
        RouterBuildError::Load(RouteLoadError::UnknownTemplate { .. })
    ));
    assert!(builder.calls.lock().unwrap().is_empty());
}
