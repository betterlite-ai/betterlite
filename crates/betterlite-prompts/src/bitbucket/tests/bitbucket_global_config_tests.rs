use betterlite_prompts::bitbucket::global::{GlobalBitBucketConfig, set_global_bitbucket_config};

use crate::support;

#[test]
fn set_stores_the_config_for_later_retrieval() {
    let global = GlobalBitBucketConfig::default();
    set_global_bitbucket_config(&global, support::config("https://api.example.test/2.0"));

    let stored = global.0.lock().expect("global config lock");
    let config = stored.as_ref().expect("the config was stored");
    let workspace = config.workspace.clone();
    let repository = config.repository.clone();
    let base_url = config.base_url.clone();
    drop(stored);

    assert_eq!(workspace.as_deref(), Some("ws"));
    assert_eq!(repository.as_deref(), Some("repo"));
    assert_eq!(base_url.as_deref(), Some("https://api.example.test/2.0"));
}

#[test]
fn a_cloned_handle_reads_the_config_set_through_the_original() {
    let global = GlobalBitBucketConfig::default();
    let shared = global.clone();
    set_global_bitbucket_config(&shared, support::config("https://api.example.test/2.0"));

    assert!(global.0.lock().expect("global config lock").is_some());
}
