use betterlite_db::budget_permission_rows::JsonData;

use crate::identity::{AgentIdentityFailure, FailureCode};

use super::AgentIdentityStore;
use super::records::{SubjectKey, VerifiedSubjectRow};

mod tests;

pub async fn find(
    store: &AgentIdentityStore,
    issuer: &str,
    tenant_id: &str,
    oid: Option<&JsonData>,
) -> Result<Option<VerifiedSubjectRow>, AgentIdentityFailure> {
    let Some(JsonData::String(subject_oid)) = oid else {
        return Ok(None);
    };
    let key = SubjectKey {
        issuer,
        tenant_id,
        oid: subject_oid,
    };
    store
        .humans
        .find_unique(&key)
        .await
        .map_err(|_source| AgentIdentityFailure {
            code: FailureCode::PolicyUnavailable,
            message: String::from("Subject classification is unavailable"),
        })
}
