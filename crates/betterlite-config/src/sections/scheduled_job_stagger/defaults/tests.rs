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

#[test]
fn config_coerces_quoted_boolean_and_integer_strings() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(
        "general_settings:\n  scheduled_job_stagger:\n    enabled: \"false\"\n    window_seconds: \"45\"\n    offsets:\n      monthly_spend_report_job: \"30\"\n",
    )
    .expect("config should load");
    let settings = &config.sections.scheduled_job_stagger;
    assert!(
        !settings.enabled,
        "enabled: \"false\" must disable staggering, not fall back to the enabled default"
    );
    assert_eq!(settings.window_seconds, 45);
    assert_eq!(settings.offsets.get("monthly_spend_report_job"), Some(&30));
}

#[test]
fn config_coerces_negative_offset_string() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("general_settings:\n  scheduled_job_stagger:\n    offsets:\n      custom_cron: \"-5\"\n")
    .expect("config should load");
    let settings = &config.sections.scheduled_job_stagger;
    assert_eq!(settings.offsets.get("custom_cron"), Some(&-5));
}

#[test]
fn config_coerces_boolean_like_integer_and_word_forms() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("general_settings:\n  scheduled_job_stagger:\n    enabled: 0\n")
    .expect("config should load");
    assert!(!config.sections.scheduled_job_stagger.enabled);

    let word_form_config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("general_settings:\n  scheduled_job_stagger:\n    enabled: \"yes\"\n")
    .expect("config should load");
    assert!(word_form_config.sections.scheduled_job_stagger.enabled);
}
