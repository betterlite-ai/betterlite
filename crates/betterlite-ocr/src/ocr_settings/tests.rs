#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use rstest::rstest;

use super::OcrSettings;

fn environment(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn an_empty_environment_keeps_the_python_defaults() {
    let empty: BTreeMap<String, String> = BTreeMap::new();
    assert_eq!(
        OcrSettings::from_environment(&empty),
        OcrSettings::default()
    );
}

#[test]
fn every_setting_follows_its_environment_variable() {
    let settings = OcrSettings::from_environment(&environment(&[
        ("REQUEST_TIMEOUT", "30.5"),
        ("MAX_IMAGE_URL_DOWNLOAD_SIZE_MB", "0.5"),
        ("AZURE_OPERATION_POLLING_TIMEOUT", " 600 "),
        ("AZURE_DOCUMENT_INTELLIGENCE_API_VERSION", "2025-01-01"),
        ("AZURE_DOCUMENT_INTELLIGENCE_DEFAULT_DPI", "72"),
    ]));
    assert_eq!(
        settings,
        OcrSettings {
            request_timeout: Duration::from_millis(30_500),
            max_download_bytes: 512 * 1024,
            poll_timeout: Duration::from_secs(600),
            doc_intelligence_api_version: "2025-01-01".to_owned(),
            doc_intelligence_dpi: 72,
            vertex_project: None,
            vertex_location: None,
            enable_azure_ad_token_refresh: false,
        }
    );
}

#[rstest]
#[case::zero_disables_downloads("0", 0)]
#[case::negative_rejects_every_download("-1", 0)]
#[case::fraction_truncates_like_int("0.0000001", 0)]
#[case::unparsable_keeps_the_default("big", 50 * 1024 * 1024)]
fn download_size_converts_megabytes_like_python(#[case] value: &str, #[case] bytes: u64) {
    let settings =
        OcrSettings::from_environment(&environment(&[("MAX_IMAGE_URL_DOWNLOAD_SIZE_MB", value)]));
    assert_eq!(settings.max_download_bytes, bytes);
}

#[test]
fn a_negative_polling_timeout_expires_immediately() {
    let settings =
        OcrSettings::from_environment(&environment(&[("AZURE_OPERATION_POLLING_TIMEOUT", "-5")]));
    assert_eq!(settings.poll_timeout, Duration::ZERO);
}

#[test]
fn an_empty_api_version_is_sent_as_is_like_python_str_of_getenv() {
    let settings = OcrSettings::from_environment(&environment(&[(
        "AZURE_DOCUMENT_INTELLIGENCE_API_VERSION",
        "",
    )]));
    assert_eq!(settings.doc_intelligence_api_version, "");
}

#[test]
fn unparsable_numbers_keep_their_defaults() {
    let defaults = OcrSettings::default();
    let settings = OcrSettings::from_environment(&environment(&[
        ("REQUEST_TIMEOUT", "soon"),
        ("AZURE_DOCUMENT_INTELLIGENCE_DEFAULT_DPI", "lots"),
    ]));
    assert_eq!(settings.request_timeout, defaults.request_timeout);
    assert_eq!(settings.doc_intelligence_dpi, defaults.doc_intelligence_dpi);
}
