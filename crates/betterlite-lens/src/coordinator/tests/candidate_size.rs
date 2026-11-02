#![cfg(test)]
use super::cluster::candidate_size;
use super::fixtures::candidate;
use crate::analysis::ObservationKind;

#[test]
fn sums_field_lengths_plus_overhead() {
    let item = candidate(
        "check",
        ObservationKind::Issue,
        "title",
        "hypothesis",
        vec!["r0"],
    );
    assert_eq!(
        candidate_size(&item),
        "title".len() + "hypothesis".len() + "check".len() + 200
    );
}

#[test]
fn counts_utf8_bytes_not_characters() {
    let item = candidate(
        "check",
        ObservationKind::Issue,
        "abc\u{e9}",
        "hypothesis",
        vec!["r0"],
    );
    assert_eq!("abc\u{e9}".chars().count(), 4);
    assert_eq!("abc\u{e9}".len(), 5);
    assert_eq!(
        candidate_size(&item),
        "abc\u{e9}".len() + "hypothesis".len() + "check".len() + 200
    );
}

#[test]
fn ignores_execution_ids_and_existing_finding_id() {
    let mut item = candidate(
        "check",
        ObservationKind::Issue,
        "title",
        "hypothesis",
        vec!["r0", "r1", "r2"],
    );
    item.existing_finding_id = Some(String::from("a-very-long-existing-finding-identifier"));
    assert_eq!(
        candidate_size(&item),
        "title".len() + "hypothesis".len() + "check".len() + 200
    );
}
