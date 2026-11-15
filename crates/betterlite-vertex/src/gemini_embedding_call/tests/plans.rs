use std::collections::BTreeMap;
use std::time::Duration;

use rstest::rstest;

use super::support::{Fixture, input, secret};
use crate::endpoint_target::target::{AuthHeader, EndpointTarget, TargetProvider};
use crate::gemini_embedding_call::call::{DEFAULT_TIMEOUT, Execution};
use crate::gemini_embedding_call::error::GeminiEmbeddingCallError;
use crate::gemini_embedding_call::plan::{
    ClientSource, Facade, Timeouts, flatten_and_detect_file_refs, plan_call,
};
use crate::gemini_embedding_call::transport::embedding_request;

#[rstest]
#[case::text(r#""hello""#, &["hello"], false)]
#[case::texts(r#"["a","b"]"#, &["a", "b"], false)]
#[case::file(r#""files/a""#, &["files/a"], true)]
#[case::nested(r#"["a",["files/b","c"]]"#, &["a", "files/b", "c"], true)]
#[case::repeated(r#"["files/a","files/a"]"#, &["files/a", "files/a"], true)]
#[case::prefix_must_lead(r#"["my files/a","Files/a","file/a"]"#, &["my files/a", "Files/a", "file/a"], false)]
#[case::empty("[]", &[], false)]
fn detects_file_references_after_flattening(
    #[case] source: &str,
    #[case] flat: &[&str],
    #[case] detected: bool,
) {
    let parsed = input(source);
    let (elements, has_file_refs) = flatten_and_detect_file_refs(&parsed);
    assert_eq!(elements, flat);
    assert_eq!(has_file_refs, detected);
}

#[rstest]
#[case::vertex(TargetProvider::Vertex, Facade::EmbedContent)]
#[case::studio(TargetProvider::Gemini, Facade::BatchEmbedContents)]
fn each_provider_uses_its_own_facade(#[case] provider: TargetProvider, #[case] facade: Facade) {
    let fixture = Fixture::new(r#""hello""#);
    let plan = plan_call(&fixture.call(provider, Execution::Sync), false).unwrap();
    assert_eq!(plan.facade, facade);
    assert!(plan.files.is_empty());
}

#[rstest]
#[case::vertex_walks_the_top_level(TargetProvider::Vertex, Some("k"), r#"["files/a",["files/b"],"c","files/a"]"#, &["files/a", "files/a"])]
#[case::vertex_without_a_key_resolves_nothing(TargetProvider::Vertex, None, r#"["files/a"]"#, &[])]
#[case::vertex_single_path(TargetProvider::Vertex, Some("k"), r#""files/a""#, &["files/a"])]
#[case::studio_flattens(TargetProvider::Gemini, Some("k"), r#"["files/a",["files/b"],"c","files/a"]"#, &["files/a", "files/b", "files/a"])]
#[case::studio_without_references(TargetProvider::Gemini, None, r#"["a",["b"]]"#, &[])]
#[case::studio_empty_key_counts_as_set(TargetProvider::Gemini, Some(""), r#""files/a""#, &["files/a"])]
fn files_to_resolve_follow_the_provider(
    #[case] provider: TargetProvider,
    #[case] key: Option<&str>,
    #[case] source: &str,
    #[case] expected: &[&str],
) {
    let mut fixture = Fixture::new(source);
    fixture.api_key = key.map(secret);
    let plan = plan_call(&fixture.call(provider, Execution::Sync), false).unwrap();
    assert_eq!(plan.files, expected);
}

#[rstest]
#[case::single(r#""files/a""#)]
#[case::nested(r#"["a",["files/b"]]"#)]
fn missing_api_key(#[case] source: &str) {
    let fixture = Fixture::new(source);
    let error = plan_call(
        &fixture.call(TargetProvider::Gemini, Execution::Sync),
        false,
    )
    .unwrap_err();
    assert!(matches!(error, GeminiEmbeddingCallError::MissingApiKey));
    assert_eq!(
        error.to_string(),
        "An API key is required to resolve Gemini file references (files/...). Pass api_key= or set GEMINI_API_KEY."
    );
}

const FALLBACK: Timeouts = Timeouts {
    read: Duration::from_secs(600),
    connect: Duration::from_secs(5),
};

#[rstest]
#[case::sync_supplied_keeps_its_own(Execution::Sync, true, None, ClientSource::Supplied, None)]
#[case::sync_supplied_ignores_the_timeout(
    Execution::Sync,
    true,
    Some(10),
    ClientSource::Supplied,
    None
)]
#[case::sync_created_without_a_timeout(
    Execution::Sync,
    false,
    None,
    ClientSource::Created,
    Some(FALLBACK)
)]
#[case::sync_created_with_a_timeout(Execution::Sync, false, Some(10), ClientSource::Created, Some(Timeouts { read: Duration::from_secs(10), connect: Duration::from_secs(10) }))]
#[case::async_never_uses_the_supplied_client(
    Execution::Async,
    true,
    None,
    ClientSource::Created,
    Some(FALLBACK)
)]
#[case::async_with_a_timeout(Execution::Async, true, Some(10), ClientSource::Created, Some(Timeouts { read: Duration::from_secs(10), connect: Duration::from_secs(10) }))]
#[case::async_without_a_supplied_client(
    Execution::Async,
    false,
    None,
    ClientSource::Created,
    Some(FALLBACK)
)]
fn client_and_timeouts_follow_the_execution(
    #[case] execution: Execution,
    #[case] supplied: bool,
    #[case] seconds: Option<u64>,
    #[case] client: ClientSource,
    #[case] timeouts: Option<Timeouts>,
) {
    let mut fixture = Fixture::new(r#""hello""#);
    fixture.timeout = seconds.map(Duration::from_secs);
    let plan = plan_call(&fixture.call(TargetProvider::Vertex, execution), supplied).unwrap();
    assert_eq!(plan.client, client);
    assert_eq!(plan.timeouts, timeouts);
}

#[test]
fn default_timeout_is_300_seconds() {
    assert_eq!(DEFAULT_TIMEOUT, Duration::from_secs(300));
}

#[test]
fn timeout_none_modes() {
    let fixture = Fixture::new(r#""hello""#);
    let created = plan_call(
        &fixture.call(TargetProvider::Vertex, Execution::Sync),
        false,
    )
    .unwrap();
    assert_eq!(created.client, ClientSource::Created);
    assert_eq!(created.timeouts, Some(FALLBACK));
    let supplied = plan_call(&fixture.call(TargetProvider::Vertex, Execution::Sync), true).unwrap();
    assert_eq!(supplied.client, ClientSource::Supplied);
    assert_eq!(supplied.timeouts, None);
    let asynchronous = plan_call(
        &fixture.call(TargetProvider::Vertex, Execution::Async),
        true,
    )
    .unwrap();
    assert_eq!(asynchronous.client, ClientSource::Created);
    assert_eq!(asynchronous.timeouts, Some(FALLBACK));
}

#[test]
fn extra_header_precedence() {
    let mut auth = BTreeMap::new();
    auth.insert(String::from("x-goog-api-key"), secret("studio-key"));
    let target = EndpointTarget {
        url: String::from("https://example.test/embed"),
        auth: Some(AuthHeader::Headers(auth)),
    };
    let extra = [
        (String::from("Content-Type"), String::from("text/plain")),
        (String::from("X-Goog-Api-Key"), String::from("override")),
    ]
    .into_iter()
    .collect::<BTreeMap<_, _>>();
    let request =
        embedding_request(&target, &extra, b"{}".to_vec(), Duration::from_secs(300)).unwrap();
    assert_eq!(request.headers["content-type"], "text/plain");
    assert_eq!(request.headers["x-goog-api-key"], "override");
}
