use std::collections::BTreeMap;

use super::support;
use crate::datadog::cost_management::datadog_cost_management::settings::{
    CostManagementInit, CostManagementSettings, DEFAULT_SITE,
};

#[test]
fn url_is_built_from_the_site_without_a_trailing_slash() {
    let settings = support::settings(&support::init(&[]));
    assert_eq!(
        settings.upload_url,
        "https://api.test.datadoghq.com/api/v2/cost/custom_costs"
    );
    assert!(!settings.upload_url.ends_with('/'));
}

#[test]
fn url_uses_the_default_site_when_dd_site_is_unset() {
    let settings = CostManagementSettings::resolve(
        &CostManagementInit::default(),
        &BTreeMap::<String, String>::new(),
    )
    .expect("the settings resolve");
    assert_eq!(settings.site, DEFAULT_SITE);
    assert_eq!(
        settings.upload_url,
        "https://api.datadoghq.com/api/v2/cost/custom_costs"
    );
}

#[test]
fn url_uses_the_default_site_when_dd_site_is_empty() {
    let environment = BTreeMap::from([("DD_SITE".to_owned(), String::new())]);
    let settings = CostManagementSettings::resolve(&CostManagementInit::default(), &environment)
        .expect("the settings resolve");
    assert_eq!(settings.site, DEFAULT_SITE);
    assert_eq!(
        settings.upload_url,
        "https://api.datadoghq.com/api/v2/cost/custom_costs"
    );
}
