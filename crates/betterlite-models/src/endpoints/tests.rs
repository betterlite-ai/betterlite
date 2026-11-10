#![cfg(test)]

use std::fs;

use rstest::rstest;

use super::projection::{clean_display_name, project};
use super::{
    EndpointSupport, MAX_REGISTRY_BYTES, MAX_REGISTRY_ENDPOINTS, PACKAGED_ASSET, ROOT_ASSET,
    RUNTIME_ASSET, RegistryAsset, RegistryError, parse, support,
};

#[derive(Clone, Copy, Debug)]
enum Check {
    PinnedRoot,
    RuntimePackaged,
    RootOnlyProvider,
    MissingVersusFalse,
    PackagedProjection,
    RootProjection,
    DisplayName,
}

#[rstest]
#[case::pinned_root_parses(Check::PinnedRoot)]
#[case::runtime_uses_the_packaged_backup(Check::RuntimePackaged)]
#[case::root_only_provider_is_unlisted_at_runtime(Check::RootOnlyProvider)]
#[case::support_keeps_missing_versus_false(Check::MissingVersusFalse)]
#[case::packaged_projection_labels_and_paths(Check::PackagedProjection)]
#[case::root_projection_covers_root_only_routes(Check::RootProjection)]
#[case::projection_cleans_provider_display_names(Check::DisplayName)]
fn model_capabilities_model_capability_tests_behavior(#[case] check: Check) {
    match check {
        Check::PinnedRoot => pinned_root(),
        Check::RuntimePackaged => runtime_packaged(),
        Check::RootOnlyProvider => root_only_provider(),
        Check::MissingVersusFalse => missing_versus_false(),
        Check::PackagedProjection => packaged_projection(),
        Check::RootProjection => root_projection(),
        Check::DisplayName => display_name(),
    }
}

fn pinned_root() {
    let root = parse(ROOT_ASSET, RegistryAsset::Root).unwrap();
    assert_eq!(root.asset, RegistryAsset::Root);
    assert_eq!(root.providers.len(), 184);
    assert_eq!(root.endpoints.len(), 38);
    assert_eq!(
        root.comment.as_deref(),
        Some("This file defines which endpoints are supported by each LiteLLM provider")
    );
    let schema = root.schema.as_ref().unwrap();
    assert!(
        schema
            .provider_slug
            .endpoints
            .contains_key("a2a_(Agent Gateway)")
    );
    assert!(root.endpoints.contains_key("sandbox"));
    assert!(root.providers.contains_key("apiserpent"));
}

fn runtime_packaged() {
    let runtime = parse(PACKAGED_ASSET, RUNTIME_ASSET).unwrap();
    assert_eq!(runtime.asset, RegistryAsset::Packaged);
    assert_eq!(runtime.providers.len(), 152);
    assert_eq!(runtime.endpoints.len(), 37);
    assert!(runtime.providers.contains_key("anthropic"));
    assert!(runtime.endpoints.contains_key("videos"));
}

fn root_only_provider() {
    let runtime = parse(PACKAGED_ASSET, RUNTIME_ASSET).unwrap();
    let root = parse(ROOT_ASSET, RegistryAsset::Root).unwrap();
    assert_eq!(
        support(&runtime, "apiserpent", "search"),
        EndpointSupport::Unlisted
    );
    assert_eq!(
        support(&root, "apiserpent", "search"),
        EndpointSupport::Supported
    );
    assert_eq!(
        support(&runtime, "e2b", "sandbox"),
        EndpointSupport::Unlisted
    );
    assert_eq!(support(&root, "e2b", "sandbox"), EndpointSupport::Supported);
}

fn missing_versus_false() {
    let runtime = parse(PACKAGED_ASSET, RUNTIME_ASSET).unwrap();
    assert_eq!(
        support(&runtime, "anthropic", "messages"),
        EndpointSupport::Supported
    );
    assert_eq!(
        support(&runtime, "abliteration", "messages"),
        EndpointSupport::Unsupported
    );
    assert_eq!(
        support(&runtime, "anthropic", "sandbox"),
        EndpointSupport::Unlisted
    );
    assert_eq!(
        support(&runtime, "not_a_provider", "messages"),
        EndpointSupport::Unlisted
    );
}

fn packaged_projection() {
    let runtime = parse(PACKAGED_ASSET, RUNTIME_ASSET).unwrap();
    let entries = project(&runtime);
    assert_eq!(entries.len(), 35);
    let first = entries.first().unwrap();
    assert_eq!(first.key, "chat_completions");
    assert_eq!(first.label, "Chat Completions");
    assert_eq!(first.endpoint, "/chat/completions");
    let first_provider = first.providers.first().unwrap();
    assert_eq!(first_provider.slug, "a2a");
    assert_eq!(first_provider.display_name, "A2A (Agent-to-Agent)");
    assert!(entries.iter().all(|entry| !entry.providers.is_empty()));
}

fn root_projection() {
    let root = parse(ROOT_ASSET, RegistryAsset::Root).unwrap();
    let entries = project(&root);
    assert_eq!(entries.len(), 37);
    let sandbox = entries.iter().find(|entry| entry.key == "sandbox").unwrap();
    assert_eq!(sandbox.label, "Sandbox");
    assert_eq!(sandbox.endpoint, "/sandbox");
    let slugs: Vec<&str> = sandbox
        .providers
        .iter()
        .map(|provider| provider.slug.as_str())
        .collect();
    assert_eq!(slugs, ["e2b", "opensandbox"]);
    let systemone = entries
        .iter()
        .find(|entry| entry.key == "systemone")
        .unwrap();
    assert_eq!(systemone.label, "Systemone");
    assert_eq!(systemone.endpoint, "/systemone");
}

fn display_name() {
    assert_eq!(
        clean_display_name("A2A (Agent-to-Agent) (`a2a`)"),
        "A2A (Agent-to-Agent)"
    );
    assert_eq!(clean_display_name("AI21 (`ai21`)"), "AI21");
    assert_eq!(clean_display_name("Plain Name"), "Plain Name");
    assert_eq!(clean_display_name("  Padded  "), "Padded");
    assert_eq!(clean_display_name("Odd (not-a-slug)"), "Odd (not-a-slug)");
    assert_eq!(clean_display_name("Odd (`a`b`)"), "Odd (`a`b`)");
}

#[test]
fn parse_rejects_an_oversized_registry() {
    let bytes = vec![b' '; MAX_REGISTRY_BYTES + 1];
    assert!(matches!(
        parse(&bytes, RegistryAsset::Packaged),
        Err(RegistryError::TooLarge { .. })
    ));
}

#[test]
fn parse_rejects_deep_nesting() {
    let source = deep();
    let error = parse(source.as_bytes(), RegistryAsset::Packaged).unwrap_err();
    assert!(matches!(error, RegistryError::TooDeep { limit: 8 }));
}

fn deep() -> String {
    let mut value = String::from("1");
    for _ in 0_i32..12_i32 {
        value = format!("{{\"a\":{value}}}");
    }
    format!("{{\"extra\":{value}}}")
}

#[test]
fn parse_rejects_a_duplicate_provider() {
    let source = r#"{"providers":{"a":{},"a":{}}}"#;
    let error = parse(source.as_bytes(), RegistryAsset::Packaged).unwrap_err();
    assert!(matches!(error, RegistryError::Json(_)));
}

#[test]
fn parse_rejects_an_unknown_field() {
    let source = r#"{"providers":{},"extra":true}"#;
    let error = parse(source.as_bytes(), RegistryAsset::Packaged).unwrap_err();
    assert!(matches!(error, RegistryError::Json(_)));
}

#[test]
fn parse_rejects_a_non_boolean_route() {
    let source = r#"{"providers":{"a":{"endpoints":{"chat_completions":"yes"}}}}"#;
    let error = parse(source.as_bytes(), RegistryAsset::Packaged).unwrap_err();
    assert!(matches!(error, RegistryError::Json(_)));
}

fn joined_entries<F>(last: usize, entry: F) -> String
where
    F: Fn(usize) -> String,
{
    (0..=last).map(entry).collect::<Vec<_>>().join(",")
}

fn many_providers() -> String {
    let entries = joined_entries(10_000, |index| format!(r#""p{index}":{{}}"#));
    format!(r#"{{"providers":{{{entries}}}}}"#)
}

fn many_provider_endpoints() -> String {
    let entries = joined_entries(MAX_REGISTRY_ENDPOINTS, |index| {
        format!(r#""e{index}":false"#)
    });
    format!(r#"{{"providers":{{"a":{{"endpoints":{{{entries}}}}}}}}}"#)
}

fn many_endpoint_labels() -> String {
    let entries = joined_entries(MAX_REGISTRY_ENDPOINTS, |index| {
        format!(
            r#""l{index}":{{"docs_label":"a","display_name":"b","leftnav_label":"c","provider_json_field":"d","url":"e"}}"#
        )
    });
    format!(r#"{{"providers":{{}},"endpoints":{{{entries}}}}}"#)
}

#[test]
fn parse_rejects_too_many_providers() {
    let error = parse(many_providers().as_bytes(), RegistryAsset::Packaged).unwrap_err();
    assert!(matches!(
        error,
        RegistryError::TooManyProviders { limit: 10_000, .. }
    ));
}

#[test]
fn parse_rejects_too_many_provider_endpoints() {
    let error = parse(
        many_provider_endpoints().as_bytes(),
        RegistryAsset::Packaged,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        RegistryError::TooManyProviderEndpoints { limit: 1_024, .. }
    ));
}

#[test]
fn parse_rejects_too_many_endpoint_labels() {
    let error = parse(many_endpoint_labels().as_bytes(), RegistryAsset::Packaged).unwrap_err();
    assert!(matches!(
        error,
        RegistryError::TooManyEndpointLabels { limit: 1_024, .. }
    ));
}

#[test]
fn replays_the_provider_capabilities_corpus() {
    fs::read_dir(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fuzz/corpus/provider_capabilities"
    ))
    .unwrap()
    .map(|path| fs::read(path.unwrap().path()).unwrap())
    .for_each(|bytes| drop(parse(&bytes, RegistryAsset::Packaged)));
}
