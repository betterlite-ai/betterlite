#![cfg(test)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_reporting::safe_proxy_config::ProxyConfigSnapshot;
use rstest::rstest;

use super::Slot;

fn load(variables: &[(&str, &str)]) -> Config {
    let environment: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader {
        lookup: environment,
    }
    .load("{}")
    .unwrap()
}

#[rstest]
#[case::unset(&[], true)]
#[case::false_value(&[("LITELLM_DISABLE_BUG_REPORT_LINK", "false")], true)]
#[case::empty(&[("LITELLM_DISABLE_BUG_REPORT_LINK", "")], true)]
#[case::true_value(&[("LITELLM_DISABLE_BUG_REPORT_LINK", "True")], false)]
fn the_slot_follows_the_disable_flag(#[case] variables: &[(&str, &str)], #[case] enabled: bool) {
    let slot = Slot::new(&load(variables));
    assert_eq!(slot.notices.enabled, enabled);
}

#[test]
fn the_slot_reads_the_dockerenv_marker() {
    let slot = Slot::new(&load(&[]));
    assert_eq!(slot.notices.dockerenv, PathBuf::from("/.dockerenv"));
}

#[test]
fn the_slot_starts_with_an_empty_config_snapshot() {
    let slot = Slot::new(&load(&[]));
    assert_eq!(slot.notices.snapshot, ProxyConfigSnapshot::default());
}
