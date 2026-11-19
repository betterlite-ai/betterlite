use std::sync::Arc;

use betterlite_models::decisions::Json;
use serde_json::Number;

use super::fixtures::{Lookups, catalog, discovery_with, limits, object, record, snapshot, spec};
use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_model_info::cache::DiscoveryCache;
use crate::deployment_model_info::configured::{
    get_configured_display_name, get_configured_mode, get_configured_token_limits,
};
use crate::deployment_model_info::discovery::{
    discovered_fields, get_discovered_model_info, merged_model_info,
};
use crate::deployment_model_info::listing::get_model_listing_info;

fn group(rows: &[(&str, &str)]) -> Arc<DeploymentCatalog> {
    let mut specs = Vec::new();
    for (params, info) in rows {
        specs.push(spec("group", params, info));
    }
    catalog(specs)
}

fn listing(
    catalog: &Arc<DeploymentCatalog>,
    discovery: &DiscoveryCache,
) -> Option<(Vec<String>, Option<i64>, Option<i64>)> {
    get_model_listing_info(&snapshot(catalog), discovery, "group")
        .unwrap()
        .map(|info| {
            (
                info.cost_map_keys,
                info.max_input_tokens,
                info.max_output_tokens,
            )
        })
}

fn configured(
    catalog: &Arc<DeploymentCatalog>,
    discovery: &DiscoveryCache,
) -> (Option<i64>, Option<i64>) {
    let lookups = Lookups::default();
    let lookup = lookups.lookup(catalog);
    get_configured_token_limits(&snapshot(catalog), &lookup, discovery, "group").unwrap()
}

#[test]
fn stale_or_wrong_cache_record() {
    let first = group(&[
        (r#"{"model":"openai/gpt-4o"}"#, r#"{"id":"one"}"#),
        (r#"{"model":"openai/gpt-4o"}"#, r#"{"id":"one"}"#),
    ]);
    let discovery = DiscoveryCache::for_deployments(2);
    discovery.store("one", record(&first, 0, limits(None, None, Some(4096))));
    let other = group(&[
        (r#"{"model":"openai/gpt-4o"}"#, r#"{"id":"one"}"#),
        (r#"{"model":"openai/gpt-4o"}"#, r#"{"id":"one"}"#),
    ]);
    assert_eq!(
        listing(&other, &discovery),
        Some((vec!["openai/gpt-4o".to_owned()], None, None))
    );
    assert!(get_discovered_model_info(&snapshot(&other), &discovery, Some("absent")).is_empty());
}

#[test]
fn equal_id_different_identity() {
    let first = group(&[(r#"{"model":"openai/gpt-4o"}"#, r#"{"id":"one"}"#)]);
    let discovery = DiscoveryCache::for_deployments(1);
    discovery.store("one", record(&first, 0, limits(None, None, Some(4096))));
    let twin = group(&[(r#"{"model":"openai/gpt-4o"}"#, r#"{"id":"one"}"#)]);
    assert_eq!(first.models, twin.models);
    assert!(!Arc::ptr_eq(&first, &twin));
    assert_eq!(
        listing(&twin, &discovery),
        Some((vec!["openai/gpt-4o".to_owned()], None, None))
    );
}

#[test]
fn configured_nonnull_suppresses_discovery() {
    let catalog = group(&[(
        r#"{"model":"openai/gpt-4o"}"#,
        r#"{"id":"one","max_output_tokens":100,"max_input_tokens":false}"#,
    )]);
    let discovery = discovery_with(&catalog, 0, limits(None, Some(2000), Some(4096)));
    assert_eq!(
        listing(&catalog, &discovery),
        Some((vec!["openai/gpt-4o".to_owned()], None, Some(100)))
    );
    assert!(get_discovered_model_info(&snapshot(&catalog), &discovery, Some("one")).is_empty());
}

#[test]
fn explicit_null_allows_discovery() {
    let configured_map = object(r#"{"max_output_tokens":null}"#);
    let fields = discovered_fields(limits(None, None, Some(4096)), &configured_map);
    assert_eq!(
        fields.get("max_output_tokens"),
        Some(&Json::Number(Number::from(4096_u64)))
    );
    let catalog = group(&[(r#"{"model":"openai/gpt-4o"}"#, r#"{"id":"one"}"#)]);
    let discovery = discovery_with(&catalog, 0, limits(None, None, Some(4096)));
    let merged = merged_model_info(
        &snapshot(&catalog),
        &discovery,
        &object(r#"{"id":"one","max_output_tokens":null}"#),
    );
    assert_eq!(
        merged.get("max_output_tokens"),
        Some(&Json::Number(Number::from(4096_u64)))
    );
}

#[test]
fn ordered_cost_key_dedup() {
    let catalog = group(&[
        (
            r#"{"model":"openai/b"}"#,
            r#"{"id":"a","base_model":"openai/gpt-4o"}"#,
        ),
        (
            r#"{"model":"openai/c","base_model":"openai/gpt-4o"}"#,
            r#"{"id":"b"}"#,
        ),
        (r#"{"model":"openai/gpt-4o-mini"}"#, r#"{"id":"c"}"#),
        (r#"{"model":"openai/d"}"#, r#"{"id":"d","base_model":""}"#),
        (r#"{"model":"openai/e"}"#, r#"{"id":"e","base_model":5}"#),
    ]);
    let discovery = DiscoveryCache::for_deployments(catalog.models.len());
    let keys = listing(&catalog, &discovery).unwrap().0;
    assert_eq!(
        keys,
        vec![
            "openai/gpt-4o".to_owned(),
            "openai/gpt-4o-mini".to_owned(),
            "openai/d".to_owned()
        ]
    );
}

#[test]
fn base_model_fallback() {
    let catalog = group(&[(r#"{"model":"openai/gpt-4o"}"#, r#"{"id":"one"}"#)]);
    let discovery = DiscoveryCache::for_deployments(1);
    assert_eq!(
        listing(&catalog, &discovery),
        Some((vec!["openai/gpt-4o".to_owned()], None, None))
    );
}

#[test]
fn unknown_and_expanded_name_absent() {
    let catalog = catalog(vec![spec(
        "openai/*",
        r#"{"model":"openai/gpt-4o"}"#,
        r#"{"id":"wild"}"#,
    )]);
    let discovery = DiscoveryCache::for_deployments(1);
    assert!(
        get_model_listing_info(&snapshot(&catalog), &discovery, "openai/gpt-4-turbo")
            .unwrap()
            .is_none()
    );
    assert!(
        get_model_listing_info(&snapshot(&catalog), &discovery, "nope")
            .unwrap()
            .is_none()
    );
}

#[test]
fn widest_group_limits() {
    let catalog = group(&[
        (
            r#"{"model":"openai/a"}"#,
            r#"{"id":"one","max_input_tokens":100,"max_output_tokens":10}"#,
        ),
        (
            r#"{"model":"openai/b"}"#,
            r#"{"id":"two","max_input_tokens":2000,"max_output_tokens":20}"#,
        ),
    ]);
    let discovery = DiscoveryCache::for_deployments(2);
    assert_eq!(
        listing(&catalog, &discovery),
        Some((
            vec!["openai/a".to_owned(), "openai/b".to_owned()],
            Some(2000),
            Some(20)
        ))
    );
}

#[test]
fn one_deployment_configured_limits() {
    let catalog = group(&[
        (
            r#"{"model":"openai/a"}"#,
            r#"{"id":"one","max_input_tokens":100}"#,
        ),
        (
            r#"{"model":"openai/b"}"#,
            r#"{"id":"two","max_input_tokens":2000}"#,
        ),
    ]);
    let discovery = DiscoveryCache::for_deployments(2);
    assert_eq!(configured(&catalog, &discovery), (Some(100), None));
}

#[test]
fn malformed_bool_nan_infinity_limits() {
    let catalog = group(&[
        (
            r#"{"model":"openai/a"}"#,
            r#"{"id":"one","max_input_tokens":true}"#,
        ),
        (
            r#"{"model":"openai/b"}"#,
            r#"{"id":"two","max_input_tokens":"nan","max_output_tokens":"Infinity"}"#,
        ),
    ]);
    let discovery = DiscoveryCache::for_deployments(2);
    assert_eq!(
        listing(&catalog, &discovery),
        Some((
            vec!["openai/a".to_owned(), "openai/b".to_owned()],
            None,
            None
        ))
    );
    assert_eq!(configured(&catalog, &discovery), (None, None));
}

#[test]
fn numeric_string_float_limits() {
    let catalog = group(&[
        (
            r#"{"model":"openai/a"}"#,
            r#"{"id":"one","max_input_tokens":"4096"}"#,
        ),
        (
            r#"{"model":"openai/b"}"#,
            r#"{"id":"two","max_input_tokens":12.9,"max_output_tokens":"20"}"#,
        ),
    ]);
    let discovery = DiscoveryCache::for_deployments(2);
    assert_eq!(listing(&catalog, &discovery).unwrap().1, Some(4096));
    assert_eq!(configured(&catalog, &discovery), (Some(4096), None));
    let single = group(&[(
        r#"{"model":"openai/a"}"#,
        r#"{"id":"one","max_output_tokens":12.9}"#,
    )]);
    let empty = DiscoveryCache::for_deployments(1);
    assert_eq!(configured(&single, &empty), (None, Some(12)));
    let text = group(&[(
        r#"{"model":"openai/a"}"#,
        r#"{"id":"one","max_output_tokens":"20"}"#,
    )]);
    assert_eq!(configured(&text, &empty), (None, Some(20)));
}

#[test]
fn zero_negative_limits() {
    let catalog = group(&[
        (
            r#"{"model":"openai/a"}"#,
            r#"{"id":"one","max_input_tokens":0}"#,
        ),
        (
            r#"{"model":"openai/b"}"#,
            r#"{"id":"two","max_input_tokens":-5}"#,
        ),
        (
            r#"{"model":"openai/c"}"#,
            r#"{"id":"three","max_output_tokens":-2}"#,
        ),
    ]);
    let discovery = DiscoveryCache::for_deployments(3);
    assert_eq!(listing(&catalog, &discovery).unwrap().1, Some(0));
    assert_eq!(configured(&catalog, &discovery), (Some(0), None));
}

#[test]
fn blank_or_nonstring_mode_and_display() {
    let catalog = group(&[(
        r#"{"model":"openai/a"}"#,
        r#"{"id":"one","mode":"  ","display_name":5}"#,
    )]);
    let lookups = Lookups::default();
    let lookup = lookups.lookup(&catalog);
    assert_eq!(get_configured_mode(&lookup, "group").unwrap(), None);
    assert_eq!(get_configured_display_name(&lookup, "group").unwrap(), None);
    let absent = Lookups::default();
    let none = absent.lookup(&catalog);
    assert_eq!(get_configured_mode(&none, "nope").unwrap(), None);
}

#[test]
fn original_whitespace_preserved() {
    let catalog = group(&[(
        r#"{"model":"openai/a"}"#,
        r#"{"id":"one","mode":" chat ","display_name":" My Name "}"#,
    )]);
    let lookups = Lookups::default();
    let lookup = lookups.lookup(&catalog);
    assert_eq!(
        get_configured_mode(&lookup, "group").unwrap(),
        Some(" chat ".to_owned())
    );
    assert_eq!(
        get_configured_display_name(&lookup, "group").unwrap(),
        Some(" My Name ".to_owned())
    );
}
