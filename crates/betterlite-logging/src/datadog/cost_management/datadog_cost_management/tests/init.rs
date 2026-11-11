use std::time::Duration;

use indexmap::IndexMap;
use rstest::rstest;

use super::support;
use crate::datadog::cost_management::datadog_cost_management::settings::{
    CostManagementInit, CostManagementSettings,
};

#[test]
fn init_defaults_cost_tag_keys_to_an_empty_list() {
    let settings =
        CostManagementSettings::resolve(&CostManagementInit::default(), &support::environment())
            .expect("the settings resolve");
    assert!(settings.cost_tag_keys.is_empty());
}

#[test]
fn init_accepts_cost_tag_keys_in_the_constructor() {
    let settings = CostManagementSettings::resolve(
        &support::init(&["capability", "tier"]),
        &support::environment(),
    )
    .expect("the settings resolve");
    assert_eq!(settings.cost_tag_keys, ["capability", "tier"]);
}

#[test]
fn init_reads_cost_tag_keys_from_the_callback_settings() {
    let params = IndexMap::from([("cost_tag_keys".to_owned(), " capability , tier ".to_owned())]);
    let init = CostManagementInit::from_params(&params).expect("the params parse");
    assert_eq!(init.cost_tag_keys, ["capability", "tier"]);
}

#[test]
fn init_reads_batch_overrides_from_the_callback_settings() {
    let params = IndexMap::from([
        ("batch_size".to_owned(), "8".to_owned()),
        ("flush_interval".to_owned(), "2.5".to_owned()),
        ("max_queue_size".to_owned(), "12".to_owned()),
    ]);
    let init = CostManagementInit::from_params(&params).expect("the params parse");
    let settings = support::settings(&init);
    assert_eq!(settings.batch.batch_size, 8);
    assert_eq!(settings.batch.flush_interval, Duration::from_millis(2_500));
    assert_eq!(settings.batch.max_queue_size, 12);
}

#[rstest]
#[case::batch_size_text("batch_size", "many")]
#[case::batch_size_negative("batch_size", "-1")]
#[case::max_queue_size_text("max_queue_size", "many")]
#[case::flush_interval_text("flush_interval", "soon")]
fn init_rejects_a_setting_that_is_not_a_count(#[case] name: &str, #[case] value: &str) {
    let params = IndexMap::from([(name.to_owned(), value.to_owned())]);
    let error = CostManagementInit::from_params(&params).expect_err("the value is invalid");
    assert!(error.to_string().contains(name));
}

#[test]
fn init_drops_empty_cost_tag_keys() {
    let params = IndexMap::from([("cost_tag_keys".to_owned(), "capability,,tier,".to_owned())]);
    let init = CostManagementInit::from_params(&params).expect("the params parse");
    assert_eq!(init.cost_tag_keys, ["capability", "tier"]);
}

#[test]
fn init_ignores_the_generic_custom_logger_settings() {
    let params = IndexMap::from([
        ("turn_off_message_logging".to_owned(), "true".to_owned()),
        ("message_logging".to_owned(), "false".to_owned()),
    ]);
    let init = CostManagementInit::from_params(&params).expect("the generic settings are accepted");
    assert!(init.cost_tag_keys.is_empty());
}

#[test]
fn init_rejects_an_unknown_setting() {
    let params = IndexMap::from([("bogus".to_owned(), "1".to_owned())]);
    let error = CostManagementInit::from_params(&params).expect_err("the setting is unknown");
    assert!(error.to_string().contains("bogus"));
}

#[test]
fn init_treats_a_zero_batch_size_and_flush_interval_as_the_defaults() {
    let params = IndexMap::from([
        ("batch_size".to_owned(), "0".to_owned()),
        ("flush_interval".to_owned(), "0".to_owned()),
    ]);
    let init = CostManagementInit::from_params(&params).expect("the params parse");
    assert_eq!(init.batch_size, None);
    assert_eq!(init.flush_interval, None);
}

#[test]
fn init_resolves_a_zero_constructor_value_to_the_defaults() {
    let init = CostManagementInit {
        cost_tag_keys: Vec::new(),
        batch_size: Some(0),
        flush_interval: Some(Duration::ZERO),
        max_queue_size: None,
    };
    let settings = support::settings(&init);
    assert_eq!(settings.batch.batch_size, 512);
    assert_eq!(settings.batch.flush_interval, Duration::from_secs(5));
}
