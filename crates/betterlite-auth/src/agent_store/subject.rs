use betterlite_db::budget_permission_rows::JsonData;

use crate::identity::{AgentIdentityFailure, FailureCode};

use super::AgentIdentityStore;
use super::records::VerifiedSubjectRow;

mod tests;

impl AgentIdentityStore {
    pub async fn subject(
        &self,
        issuer: &str,
        tenant_id: &str,
        oid: Option<&JsonData>,
    ) -> Result<Option<VerifiedSubjectRow>, AgentIdentityFailure> {
        let Some(JsonData::String(oid)) = oid else {
            return Ok(None);
        };
        self.humans
            .find_unique(issuer, tenant_id, oid)
            .await
            .map_err(|_source| AgentIdentityFailure {
                code: FailureCode::PolicyUnavailable,
                message: String::from("Subject classification is unavailable"),
            })
    }
}
