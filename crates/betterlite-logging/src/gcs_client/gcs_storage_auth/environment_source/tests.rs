#![cfg(test)]

use std::collections::BTreeMap;

use super::EnvironmentSource;

fn environment() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("GCS_BUCKET_NAME".to_owned(), "bucket".to_owned()),
        ("GCS_PATH_SERVICE_ACCOUNT".to_owned(), "account".to_owned()),
    ])
}

#[test]
fn a_set_variable_is_read_by_name() {
    let source = environment();
    assert_eq!(
        EnvironmentSource::get(&source, "GCS_BUCKET_NAME"),
        Some("bucket".to_owned())
    );
    assert_eq!(
        EnvironmentSource::get(&source, "GCS_PATH_SERVICE_ACCOUNT"),
        Some("account".to_owned())
    );
}

#[test]
fn a_variable_that_is_not_set_reads_as_none() {
    let source = environment();
    assert_eq!(EnvironmentSource::get(&source, "GCS_MOCK"), None);
}

#[test]
fn the_snapshot_holds_every_variable() {
    let source = environment();
    assert_eq!(EnvironmentSource::snapshot(&source), environment());
}
