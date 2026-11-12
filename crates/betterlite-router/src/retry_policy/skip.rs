use crate::retry_policy::eligibility::is_retryable_status;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FailedDeployment<'names> {
    pub retry_skip_deployment_id: Option<&'names str>,
    pub failed_deployment_id: Option<&'names str>,
}

// a deployment that answered a non-retryable status is skipped for the rest of the request's retries
#[must_use]
pub fn skipped_deployment_ids(
    status: u16,
    failed: FailedDeployment<'_>,
    already_skipped: &[String],
) -> Vec<String> {
    let Some(deployment) = failed
        .retry_skip_deployment_id
        .or(failed.failed_deployment_id)
    else {
        return Vec::new();
    };
    if is_retryable_status(status) {
        return Vec::new();
    }
    let mut skipped = already_skipped.to_vec();
    skipped.push(deployment.to_owned());
    skipped.sort();
    skipped.dedup();
    skipped
}
