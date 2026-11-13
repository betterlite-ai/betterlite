use betterlite_auth::auth_identity::records::Principal;
use betterlite_core::api::managed_resource_store::is_proxy_admin;

use crate::records::TranscribeRefusal;

const PRICED_OPERATION: &str = "StartTranscriptionJob";
const OWNED_JOB_OPERATIONS: [&str; 2] = ["GetTranscriptionJob", "DeleteTranscriptionJob"];

#[must_use]
pub fn transcribe_admin_only_refusal(
    operation: &str,
    principal: Option<&Principal>,
) -> Option<TranscribeRefusal> {
    if operation == PRICED_OPERATION
        || OWNED_JOB_OPERATIONS.contains(&operation)
        || is_proxy_admin(principal)
    {
        return None;
    }
    Some(TranscribeRefusal {
        status_code: 403,
        detail: format!(
            "{operation} can access every Transcribe resource in the AWS account; only a proxy admin may call it"
        ),
    })
}
