use crate::db_lifecycle::DatabaseFailure;

#[derive(Debug, thiserror::Error)]
#[error("a plain failure")]
struct PlainFailure;

impl DatabaseFailure for PlainFailure {}

#[test]
fn the_default_failure_classification_is_both_kinds() {
    let failure = PlainFailure;
    assert!(failure.is_connection_failure());
    assert!(failure.is_infrastructure_failure());
}
