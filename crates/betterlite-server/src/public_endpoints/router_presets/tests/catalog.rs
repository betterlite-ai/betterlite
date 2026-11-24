use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{REMOTE_CATALOG, bundled_catalog};
use crate::public_endpoints::router_presets::catalog::{MAX_CATALOG_BYTES, decode_catalog};
use crate::public_endpoints::router_presets::error::RouterPresetsError;

#[test]
fn decodes_the_bundled_catalog() {
    let catalog = bundled_catalog();
    let names: Vec<&str> = catalog.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        [
            "1m_context",
            "anthropic_family",
            "gemini_family",
            "lite",
            "openai_family"
        ]
    );
    let preset = catalog.get("1m_context").unwrap();
    assert_eq!(preset.label, "1M Context");
    assert_eq!(preset.complexity_router_config.tiers.simple, ["gpt-6-luna"]);
    assert_eq!(
        preset.complexity_router_config.tiers.reasoning,
        ["claude-opus-5-5"]
    );
}

#[test]
fn keeps_extra_fields_on_records_and_configs() {
    let catalog = decode_catalog(REMOTE_CATALOG.as_bytes()).unwrap();
    let preset = catalog.get("custom").unwrap();
    assert_eq!(
        preset.extra.get("badge"),
        Some(&Json::Text("gold".to_owned()))
    );
    assert_eq!(
        preset.complexity_router_config.extra.get("classifier_type"),
        Some(&Json::Text("heuristic".to_owned()))
    );
    let bundled = bundled_catalog();
    let lite = bundled.get("lite").unwrap();
    assert_eq!(
        lite.complexity_router_config.extra.get("session_affinity"),
        Some(&Json::Bool(false))
    );
    let window = lite
        .complexity_router_config
        .extra
        .get("classifier_context_window_size")
        .unwrap();
    assert_eq!(serde_json::to_string(window).unwrap(), "0");
}

#[rstest]
#[case::unknown_tier(
    r#"{"p":{"label":"P","description":"D","complexity_router_config":{"tiers":{"SIMPLE":[],"MEDIUM":[],"COMPLEX":[],"REASONING":[],"EXTRA":[]}}}}"#
)]
#[case::missing_simple(
    r#"{"p":{"label":"P","description":"D","complexity_router_config":{"tiers":{"MEDIUM":[],"COMPLEX":[],"REASONING":[]}}}}"#
)]
#[case::missing_reasoning(
    r#"{"p":{"label":"P","description":"D","complexity_router_config":{"tiers":{"SIMPLE":[],"MEDIUM":[],"COMPLEX":[]}}}}"#
)]
#[case::tier_not_a_list(
    r#"{"p":{"label":"P","description":"D","complexity_router_config":{"tiers":{"SIMPLE":"small","MEDIUM":[],"COMPLEX":[],"REASONING":[]}}}}"#
)]
#[case::tier_model_not_a_string(
    r#"{"p":{"label":"P","description":"D","complexity_router_config":{"tiers":{"SIMPLE":[1],"MEDIUM":[],"COMPLEX":[],"REASONING":[]}}}}"#
)]
#[case::tiers_null(
    r#"{"p":{"label":"P","description":"D","complexity_router_config":{"tiers":null}}}"#
)]
#[case::missing_label(r#"{"p":{"description":"D","complexity_router_config":{"tiers":{"SIMPLE":[],"MEDIUM":[],"COMPLEX":[],"REASONING":[]}}}}"#)]
#[case::label_not_a_string(r#"{"p":{"label":7,"description":"D","complexity_router_config":{"tiers":{"SIMPLE":[],"MEDIUM":[],"COMPLEX":[],"REASONING":[]}}}}"#)]
#[case::missing_config(r#"{"p":{"label":"P","description":"D"}}"#)]
#[case::record_not_an_object(r#"{"p":"text"}"#)]
#[case::top_level_not_an_object("[]")]
#[case::top_level_null("null")]
#[case::not_json("not a catalog")]
#[case::trailing_characters("{} {}")]
fn rejects_catalogs_that_do_not_match_the_wire_shape(#[case] text: &str) {
    assert!(matches!(
        decode_catalog(text.as_bytes()),
        Err(RouterPresetsError::Decode { .. })
    ));
}

#[test]
fn an_empty_object_decodes_to_an_empty_catalog() {
    assert!(decode_catalog(b"{}").unwrap().is_empty());
}

#[test]
fn rejects_a_catalog_over_the_size_limit() {
    let bytes = vec![b' '; MAX_CATALOG_BYTES + 1];
    assert!(matches!(
        decode_catalog(&bytes),
        Err(RouterPresetsError::Oversize { .. })
    ));
}

#[test]
fn accepts_a_catalog_exactly_at_the_size_limit() {
    let mut bytes = vec![b' '; MAX_CATALOG_BYTES];
    bytes[0] = b'{';
    bytes[MAX_CATALOG_BYTES - 1] = b'}';
    assert!(decode_catalog(&bytes).unwrap().is_empty());
}

#[test]
fn accepts_a_catalog_larger_than_two_kilobytes() {
    let mut bytes = br#"{"p":{"label":"P","description":"D","complexity_router_config":{"tiers":{"SIMPLE":[],"MEDIUM":[],"COMPLEX":[],"REASONING":[]}}}}"#.to_vec();
    bytes.resize(3 * 1024, b' ');
    assert_eq!(decode_catalog(&bytes).unwrap().len(), 1);
}
