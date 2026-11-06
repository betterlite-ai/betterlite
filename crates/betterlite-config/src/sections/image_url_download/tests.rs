#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::Settings;
use crate::loader::{ConfigError, ConfigLoader};

fn load(environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load("")?
        .sections
        .image_url_download)
}

#[rstest]
#[case::default("", Some(50.0_f64))]
#[case::set_value("2.5", Some(2.5_f64))]
#[case::zero("0", Some(0.0_f64))]
#[case::spaces("  0.5  ", Some(0.5_f64))]
#[case::scientific("1e-05", Some(1e-5_f64))]
#[case::empty_is_unset("   ", Some(50.0_f64))]
#[case::infinity("inf", Some(f64::INFINITY))]
#[case::negative_infinity("-inf", Some(f64::NEG_INFINITY))]
#[case::nan("nan", Some(f64::NAN))]
#[case::bad_value("fifty", None)]
fn download_limit_follows_litellm(#[case] value: &str, #[case] expected: Option<f64>) {
    let loaded = load(&[("MAX_IMAGE_URL_DOWNLOAD_SIZE_MB", value)]);
    let Some(wanted) = expected else {
        let error = loaded.unwrap_err();
        assert!(matches!(error, ConfigError::InvalidFloat { .. }));
        assert!(error.to_string().contains("MAX_IMAGE_URL_DOWNLOAD_SIZE_MB"));
        return;
    };
    let settings = loaded.unwrap();
    if wanted.is_nan() {
        assert!(settings.max_download_size_mb.is_nan());
    } else {
        assert_eq!(settings.max_download_size_mb.to_bits(), wanted.to_bits());
    }
}

#[test]
fn settings_default_matches_the_loaded_default() {
    let loaded = load(&[]).unwrap();
    assert_eq!(Settings::default(), loaded);
}
