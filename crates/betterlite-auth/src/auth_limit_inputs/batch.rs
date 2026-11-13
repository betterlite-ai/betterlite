use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::budget_permission_rows::JsonData;
use serde::Deserialize;

use crate::resolved_key::auth::UserAPIKeyAuth;
use crate::user_roles::UserRole;

use super::error::{AuthLimitInputsError, ManagementEntity};
use super::estimates::{present_value, same_value};

pub const BATCH_ENQUEUED_TOKEN_LIMIT_METADATA_KEY: &str = "batch_enqueued_token_limit";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum BatchEnqueuedTokenLimitField {
    Metadata,
}

// The request members the batch admin gate reads, with model_fields_set saying whether metadata was sent.
#[derive(Clone, Copy, Debug)]
pub struct BatchEnqueuedTokenLimitRequest<'request> {
    pub metadata: Option<&'request BTreeMap<String, JsonData>>,
    pub model_fields_set: &'request BTreeSet<BatchEnqueuedTokenLimitField>,
}

pub fn enforce_batch_enqueued_token_limit_is_admin_only(
    request: &BatchEnqueuedTokenLimitRequest<'_>,
    existing_metadata: Option<&BTreeMap<String, JsonData>>,
    caller: &UserAPIKeyAuth,
    entity: ManagementEntity,
) -> Result<(), AuthLimitInputsError> {
    if caller.extra.user_role == Some(UserRole::ProxyAdmin) {
        return Ok(());
    }
    let empty = BTreeMap::new();
    let stored = existing_metadata.unwrap_or(&empty);
    let requested = if request
        .model_fields_set
        .contains(&BatchEnqueuedTokenLimitField::Metadata)
    {
        request.metadata.unwrap_or(&empty)
    } else {
        stored
    };
    let requested_limit = requested
        .get(BATCH_ENQUEUED_TOKEN_LIMIT_METADATA_KEY)
        .and_then(present_value);
    let stored_limit = stored
        .get(BATCH_ENQUEUED_TOKEN_LIMIT_METADATA_KEY)
        .and_then(present_value);
    if same_value(requested_limit.as_ref(), stored_limit.as_ref()) {
        return Ok(());
    }
    Err(AuthLimitInputsError::BatchEnqueuedTokenLimitAdminOnly { entity })
}
