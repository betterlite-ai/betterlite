use betterlite_lens::models::execution::Sample;
use betterlite_lens::models::finding::FindingDraft;
use betterlite_lens::models::job::Lens;
use betterlite_lens::sources::storage::Storage;

use crate::lens_access::storage::source_reader;
use crate::lens_worker_routes::error::LensWorkerError;

// inference.validate_finding: the same-check rule first, then every quote checked against the stored span
pub(crate) async fn validate_finding<Store>(
    lens: &Lens,
    selected: &Sample,
    finding: &FindingDraft,
    storage: Option<&Store>,
) -> Result<(), LensWorkerError<<Store as Storage>::Error>>
where
    Store: Storage + Clone + Send + Sync,
{
    let previous = finding
        .existing_finding_id
        .as_ref()
        .and_then(|wanted| lens.findings.iter().find(|existing| existing.id == *wanted));
    let same_check = previous.is_some_and(|existing| existing.check_id == finding.check_id);
    if finding.existing_finding_id.is_some() && !same_check {
        return Err(LensWorkerError::ExistingCheck);
    }
    for evidence in &finding.evidence {
        let reader = source_reader(storage)?;
        let execution = selected
            .executions
            .iter()
            .find(|entry| entry.id == evidence.execution_id)
            .ok_or(LensWorkerError::MissingRun)?;
        if !reader
            .verify_evidence(&lens.scope, execution, evidence)
            .await?
        {
            return Err(LensWorkerError::EvidenceMismatch);
        }
    }
    Ok(())
}

mod tests;
