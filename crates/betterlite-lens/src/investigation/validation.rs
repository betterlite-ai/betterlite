use crate::analysis::primitives::evidence_valid;
use crate::analysis::{AnalysisError, Candidate, Decision, DecisionAction, ObservationKind};
use crate::models::execution::TracePart;
use crate::models::finding::{FindingDraft, Kind, Role};
use crate::models::messages::Claim;
use crate::trace_store::TraceStore;

pub(super) fn valid_finding(
    claim: &Claim,
    candidate: &Candidate,
    decision: &Decision,
    evidence: &[TracePart],
    store: &TraceStore,
) -> Result<Option<FindingDraft>, AnalysisError> {
    let Some(finding) = (decision.action == DecisionAction::Submit)
        .then_some(decision.finding.as_ref())
        .flatten()
        .cloned()
    else {
        return Ok(None);
    };
    let known = claim.job.settings.analysis_checks();
    let existing = finding
        .existing_finding_id
        .as_ref()
        .and_then(|id| claim.findings.iter().find(|item| &item.id == id));
    let existing_valid = finding.existing_finding_id.is_none()
        || existing.is_some_and(|item| item.check_id == finding.check_id);
    let expected_kind = match candidate.kind {
        ObservationKind::Issue => Kind::Issue,
        ObservationKind::Pattern => Kind::Pattern,
    };
    let evidence_is_valid =
        finding
            .evidence
            .iter()
            .try_fold(true, |valid, item| -> Result<bool, AnalysisError> {
                if !valid || evidence_valid(item, evidence) {
                    Ok(valid)
                } else {
                    Ok(store.evidence(item)?.is_some())
                }
            })?;
    let valid = known.iter().any(|check| check.id == finding.check_id)
        && finding.check_id == candidate.check_id
        && finding.kind == expected_kind
        && finding
            .evidence
            .iter()
            .any(|item| item.role == Role::Support)
        && existing_valid
        && evidence_is_valid;
    Ok(valid.then_some(finding))
}
