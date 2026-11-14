#![cfg(test)]

use betterlite_roi::estimator::evidence::metadata_evidence;
use betterlite_roi::estimator::json::json_dumps_text;

use super::support::{EVIDENCE, pull};

#[test]
fn metadata_evidence_maps_pull_fields_in_litellm_order() {
    let evidence = metadata_evidence(&pull());
    assert_eq!(json_dumps_text(&evidence).unwrap(), EVIDENCE);
}

#[test]
fn metadata_evidence_leaves_out_fields_the_litellm_model_does_not_carry() {
    let evidence = json_dumps_text(&metadata_evidence(&pull())).unwrap();
    assert!(!evidence.contains("\"url\""));
    assert!(!evidence.contains("\"login\""));
    assert!(!evidence.contains("\"emails\""));
    assert!(!evidence.contains("\"incomplete_metadata\""));
}

#[test]
fn metadata_evidence_omits_unset_commit_totals_and_keeps_explicit_nulls() {
    let evidence = metadata_evidence(&pull());
    assert_eq!(evidence.commits[0].additions, Some(5));
    assert_eq!(evidence.commits[0].deletions, None);
    assert_eq!(evidence.commits[1].changed_files, Some(None));
}
