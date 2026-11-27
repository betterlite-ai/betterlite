use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_db::identity_rows::key::{DeletedVerificationToken, LiteLLMVerificationToken};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::audit_events::identity::CallerIdentity;

use super::clock::ArchiveClock;

#[must_use]
pub fn transform_verification_tokens_to_deleted_records(
    keys: &[LiteLLMVerificationToken],
    caller: &CallerIdentity,
    litellm_changed_by: Option<&str>,
    clock: &dyn ArchiveClock,
) -> Vec<DeletedVerificationToken> {
    if keys.is_empty() {
        return Vec::new();
    }
    // one instant for the whole batch, as LiteLLM reads datetime.now once before its loop
    let deleted_at = clock.now();
    keys.iter()
        .map(|key| deleted_record(key, caller, litellm_changed_by, deleted_at))
        .collect()
}

fn deleted_record(
    key: &LiteLLMVerificationToken,
    caller: &CallerIdentity,
    litellm_changed_by: Option<&str>,
    deleted_at: DateTime<Utc>,
) -> DeletedVerificationToken {
    let mut token = key.clone();
    // LiteLLM pops org_id and writes the value under organization_id only when it was set
    let organization_id = token.org_id.take();
    token.litellm_budget_table = None;
    token.object_permission = None;
    token.budget_limits = None;
    DeletedVerificationToken {
        token,
        id: Some(Uuid::new_v4().to_string()),
        organization_id,
        deleted_at: Some(Timestamp::Naive(deleted_at.naive_utc())),
        deleted_by: caller.user_id.clone(),
        deleted_by_api_key: caller.api_key.clone(),
        litellm_changed_by: litellm_changed_by.map(str::to_owned),
    }
}

mod tests;
