#![cfg(test)]

use rstest::rstest;

use super::{CatalogProvenance, CatalogSource, SourceKind};

#[rstest]
#[case::local(true, "https://catalog.test/model.json", CatalogSource::Local)]
#[case::remote(
    false,
    "https://catalog.test/model.json",
    CatalogSource::Remote { url: "https://catalog.test/model.json".to_owned() }
)]
fn catalog_refresh_tests_behavior(
    #[case] local: bool,
    #[case] url: &str,
    #[case] expected: CatalogSource,
) {
    assert_eq!(CatalogSource::from_settings(local, url), expected);
}

#[test]
fn a_fresh_provenance_reports_no_load() {
    let provenance = CatalogProvenance::default();
    assert_eq!(provenance.kind, SourceKind::Local);
    assert!(!provenance.is_env_forced);
    assert!(provenance.loaded_at.is_none());
    assert!(provenance.source_revision.is_none());
    assert!(provenance.etag.is_none());
    assert!(provenance.fallback_reason.is_none());
    assert!(provenance.url.is_none());
}
