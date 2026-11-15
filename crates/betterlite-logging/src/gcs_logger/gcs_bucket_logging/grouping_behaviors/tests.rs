#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::super::config_key_behaviors::ConfigKeyError;
use super::super::tests::support::{Answer, dynamic, item, open};
use super::group_items_by_config;

#[tokio::test]
async fn items_with_the_same_config_are_grouped() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    let items = vec![
        item("chatcmpl-1", None, dynamic(&[("gcs_bucket_name", "a")])),
        item("chatcmpl-2", None, dynamic(&[("gcs_bucket_name", "b")])),
        item("chatcmpl-3", None, dynamic(&[("gcs_bucket_name", "a")])),
    ];
    let grouped = group_items_by_config(&harness.logger, &items).expect("the items group");
    assert_eq!(grouped.len(), 2);
    assert_eq!(grouped["a|default"].len(), 2);
    assert_eq!(grouped["a|default"][0].payload.id, "chatcmpl-1");
    assert_eq!(grouped["a|default"][1].payload.id, "chatcmpl-3");
    assert_eq!(grouped["b|default"].len(), 1);
}

#[tokio::test]
async fn groups_keep_their_first_appearance_order() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    let items = vec![
        item("chatcmpl-1", None, dynamic(&[("gcs_bucket_name", "b")])),
        item("chatcmpl-2", None, dynamic(&[("gcs_bucket_name", "a")])),
        item("chatcmpl-3", None, dynamic(&[("gcs_bucket_name", "b")])),
    ];
    let grouped = group_items_by_config(&harness.logger, &items).expect("the items group");
    let keys: Vec<&str> = grouped.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["b|default", "a|default"]);
}

#[tokio::test]
async fn a_non_string_dynamic_value_is_refused() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    let dynamic_params = Json::Object(IndexMap::from([(
        "gcs_bucket_name".to_owned(),
        Json::Bool(true),
    )]));
    let items = vec![item("chatcmpl-1", None, dynamic_params)];
    let error =
        group_items_by_config(&harness.logger, &items).expect_err("the bucket must be a string");
    assert!(matches!(error, ConfigKeyError::Field { .. }));
}

#[tokio::test]
async fn an_empty_batch_groups_to_nothing() {
    let harness = open(&[], Some("bucket"), Answer::ok("{}"));
    let grouped = group_items_by_config(&harness.logger, &[]).expect("no items group");
    assert!(grouped.is_empty());
}
