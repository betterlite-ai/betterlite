#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader, EnvironmentLookup};
use crate::sections::file_upload_options::Settings;

fn load(source: &str) -> Result<Settings, ConfigError> {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .map(|config| config.sections.file_upload_options)
}

#[rstest]
#[case::omitted("{}", false)]
#[case::disabled("litellm_settings: {require_managed_files: false}", false)]
#[case::enabled("litellm_settings: {require_managed_files: true}", true)]
#[case::null("litellm_settings: {require_managed_files: null}", false)]
#[case::zero("litellm_settings: {require_managed_files: 0}", false)]
#[case::one("litellm_settings: {require_managed_files: 1}", false)]
#[case::text("litellm_settings: {require_managed_files: 'true'}", false)]
fn settings_loader(#[case] source: &str, #[case] expected: bool) {
    assert_eq!(load(source).unwrap().require_managed_files, expected);
}

struct ManagedOnlyEnv;

impl EnvironmentLookup for ManagedOnlyEnv {
    fn lookup(&self, name: &str) -> Option<String> {
        name.contains("MANAGED").then(|| "true".to_owned())
    }
}

#[test]
fn no_environment_value_enables_it() {
    let source = "litellm_settings: {require_managed_files: os.environ/REQUIRE_MANAGED_FILES}";
    let config = ConfigLoader {
        lookup: ManagedOnlyEnv,
    }
    .load(source)
    .unwrap();
    assert!(!config.sections.file_upload_options.require_managed_files);
}
