#![cfg(test)]

use crate::loader::ConfigLoader;
use std::collections::BTreeMap;

#[rstest::rstest]
#[case::loads_allow_legacy_cloud_file_ids(
    "model_list:\n  - model_name: x\n    litellm_params:\n      allow_legacy_cloud_file_ids: true\n"
)]
fn cloud_storage_config_cases(#[case] source: &str) {
    drop(
        ConfigLoader {
            lookup: BTreeMap::<String, String>::new(),
        }
        .load(source)
        .unwrap(),
    );
}
