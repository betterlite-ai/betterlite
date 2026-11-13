use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;

use crate::resolved_key::auth::UserAPIKeyAuth;

use super::key_limits::{ModelLimits, ModelRateLimitKey, integer_limits};

const MCP_RPM_LIMIT_FIELD: &str = "mcp_rpm_limit";
const TAG_RPM_LIMIT_FIELD: &str = "tag_rpm_limit";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetadataAccessor {
    Team,
    Organization,
    Project,
}

impl MetadataAccessor {
    fn metadata(self, auth: &UserAPIKeyAuth) -> Option<&BTreeMap<String, JsonData>> {
        match self {
            Self::Team => auth.view.team_metadata.as_ref().map(|payload| &payload.0),
            Self::Organization => auth
                .view
                .organization_metadata
                .as_ref()
                .map(|payload| &payload.0),
            Self::Project => auth
                .view
                .project_metadata
                .as_ref()
                .map(|payload| &payload.0),
        }
    }
}

// The requested tenant map is read as stored: no merge, no validation, and no fallback to another owner.
#[must_use]
pub fn get_model_rate_limit_from_metadata(
    auth: &UserAPIKeyAuth,
    accessor: MetadataAccessor,
    rate_limit_key: ModelRateLimitKey,
) -> Option<ModelLimits> {
    let metadata = accessor.metadata(auth)?;
    if metadata.is_empty() {
        return None;
    }
    match metadata.get(rate_limit_key.field()) {
        None | Some(JsonData::Null) => None,
        Some(value) => integer_limits(value),
    }
}

#[must_use]
pub fn get_key_mcp_rpm_limit(auth: &UserAPIKeyAuth) -> Option<ModelLimits> {
    let metadata = &auth.view.token.metadata.0;
    if !metadata.is_empty() {
        match metadata.get(MCP_RPM_LIMIT_FIELD) {
            None | Some(JsonData::Null) => {}
            Some(value) => return integer_limits(value),
        }
    }
    get_team_mcp_rpm_limit(auth)
}

#[must_use]
pub fn get_team_mcp_rpm_limit(auth: &UserAPIKeyAuth) -> Option<ModelLimits> {
    let payload = auth.view.team_metadata.as_ref()?;
    let metadata = &payload.0;
    if metadata.is_empty() {
        return None;
    }
    match metadata.get(MCP_RPM_LIMIT_FIELD) {
        None | Some(JsonData::Null) => None,
        Some(value) => integer_limits(value),
    }
}

#[must_use]
pub fn get_key_tag_rpm_limit(auth: &UserAPIKeyAuth) -> Option<ModelLimits> {
    let metadata = &auth.view.token.metadata.0;
    if metadata.is_empty() {
        return None;
    }
    match metadata.get(TAG_RPM_LIMIT_FIELD) {
        None | Some(JsonData::Null) => None,
        Some(value) => integer_limits(value),
    }
}
