#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;

#[test]
fn config_absent_uses_defaults() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("model_list: []\n")
    .expect("config should load");
    let settings = &config.sections.scheduled_job_stagger;
    assert!(settings.enabled);
    assert_eq!(settings.window_seconds, 300);
    assert_eq!(settings.identity, None);
    assert!(settings.offsets.is_empty());
}

#[test]
fn config_reads_every_field() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(
        "general_settings:\n  scheduled_job_stagger:\n    enabled: false\n    window_seconds: 120\n    identity: worker-7\n    offsets:\n      monthly_spend_report_job: 30\n",
    )
    .expect("config should load");
    let settings = &config.sections.scheduled_job_stagger;
    assert!(!settings.enabled);
    assert_eq!(settings.window_seconds, 120);
    assert_eq!(settings.identity.as_deref(), Some("worker-7"));
    assert_eq!(settings.offsets.get("monthly_spend_report_job"), Some(&30));
}
