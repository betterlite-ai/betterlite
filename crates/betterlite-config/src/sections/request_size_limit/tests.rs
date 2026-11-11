#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::request_size_limit::Settings;

fn load(source: &str) -> Result<Settings, ConfigError> {
    Ok(ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)?
    .sections
    .request_size_limit)
}

fn load_with_environment(
    source: &str,
    environment: &[(&str, &str)],
) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .request_size_limit)
}

#[rstest]
#[case::empty("{}")]
#[case::absent("general_settings:\n  master_key: sk-test\n")]
#[case::explicit_null("general_settings:\n  max_request_size_mb: null\n")]
#[case::tilde_null("general_settings:\n  max_request_size_mb: ~\n")]
fn max_request_size_is_unset(#[case] source: &str) {
    assert_eq!(load(source).unwrap(), Settings::default());
}

#[rstest]
#[case::integer("general_settings:\n  max_request_size_mb: 5\n", 5.0_f64)]
#[case::float("general_settings:\n  max_request_size_mb: 0.5\n", 0.5_f64)]
#[case::negative("general_settings:\n  max_request_size_mb: -2\n", -2.0_f64)]
#[case::zero("general_settings:\n  max_request_size_mb: 0\n", 0.0_f64)]
fn max_request_size_reads_a_number(#[case] source: &str, #[case] expected: f64) {
    assert_eq!(load(source).unwrap().max_request_size_mb, Some(expected));
}

#[rstest]
#[case::text("max_request_size_mb: large")]
#[case::boolean("max_request_size_mb: true")]
#[case::list("max_request_size_mb: [1, 2]")]
#[case::map("max_request_size_mb: {size: 1}")]
#[case::infinite("max_request_size_mb: .inf")]
#[case::not_a_number("max_request_size_mb: .nan")]
fn max_request_size_rejects_a_non_number(#[case] setting: &str) {
    let source = format!("general_settings:\n  {setting}\n");
    assert!(matches!(
        load(&source).unwrap_err(),
        ConfigError::Yaml { .. }
    ));
}

#[test]
fn max_request_size_has_no_root_level_spelling() {
    let error = load("max_request_size_mb: 5\n").unwrap_err();
    let ConfigError::Unsupported { paths } = &error else {
        panic!("expected an unsupported key, got {error:?}");
    };
    assert!(
        paths.iter().any(|path| path == "max_request_size_mb"),
        "{paths:?}"
    );
}

#[test]
fn max_request_size_has_no_environment_spelling() {
    let settings = load_with_environment("{}", &[("MAX_REQUEST_SIZE_MB", "5")]).unwrap();
    assert_eq!(settings.max_request_size_mb, None);
}
