#![cfg(test)]
use super::RawSetting;
use crate::loader::ConfigLoader;
use serde::de::IgnoredAny;
use std::collections::BTreeMap;
#[rstest::rstest]
#[case::loads_allow_legacy_cloud_file_ids("", RawSetting::Boolean(false))]
#[case::loads_allow_legacy_cloud_file_ids_bool(
    "allow_legacy_cloud_file_ids: true",
    RawSetting::Boolean(true)
)]
#[case::loads_allow_legacy_cloud_file_ids_yes("allow_legacy_cloud_file_ids: ' YeS '", RawSetting::Text(" YeS ".to_owned()))]
#[case::loads_allow_legacy_cloud_file_ids_no("allow_legacy_cloud_file_ids: 'false'", RawSetting::Text("false".to_owned()))]
#[case::loads_allow_legacy_cloud_file_ids_integer(
    "allow_legacy_cloud_file_ids: 1",
    RawSetting::Other(IgnoredAny)
)]
fn cloud_storage_config_cases(#[case] field: &str, #[case] expected: RawSetting) {
    let source = format!(
        "model_list:\n- model_name: a\n  litellm_params: {{{field}}}\n- model_name: b\n  litellm_params: {{}}\nlitellm_settings: {{allow_legacy_cloud_file_ids: true}}"
    );
    let loader = ConfigLoader {
        lookup: BTreeMap::from([("ALLOW_LEGACY_CLOUD_FILE_IDS".to_owned(), "true".to_owned())]),
    };
    let config = loader.load(&source).unwrap();
    assert_eq!(
        config.sections.cloud_storage.deployment_legacy,
        [expected, RawSetting::Boolean(false)]
    );
    assert_eq!(
        config.sections.cloud_storage.global_legacy,
        Some(RawSetting::Boolean(true))
    );
    loader
        .load("litellm_settings: {unknown_cloud_flag: true}")
        .unwrap_err();
}
