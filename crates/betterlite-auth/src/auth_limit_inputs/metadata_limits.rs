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

fn limit_from(metadata: &BTreeMap<String, JsonData>, field: &str) -> Option<ModelLimits> {
    if metadata.is_empty() {
        return None;
    }
    match metadata.get(field) {
        None | Some(JsonData::Null) => None,
        Some(value) => integer_limits(value),
    }
}

// The requested tenant map is read as stored: no merge, no validation, and no fallback to another owner.
#[must_use]
pub fn model_limit_from_metadata(
    auth: &UserAPIKeyAuth,
    accessor: MetadataAccessor,
    rate_limit_key: ModelRateLimitKey,
) -> Option<ModelLimits> {
    match accessor.metadata(auth) {
        Some(metadata) => limit_from(metadata, rate_limit_key.field()),
        None => None,
    }
}

#[must_use]
pub fn get_key_mcp_rpm_limit(auth: &UserAPIKeyAuth) -> Option<ModelLimits> {
    if let Some(limits) = limit_from(&auth.view.token.metadata.0, MCP_RPM_LIMIT_FIELD) {
        return Some(limits);
    }
    get_team_mcp_rpm_limit(auth)
}

#[must_use]
pub fn get_team_mcp_rpm_limit(auth: &UserAPIKeyAuth) -> Option<ModelLimits> {
    match auth.view.team_metadata.as_ref() {
        Some(payload) => limit_from(&payload.0, MCP_RPM_LIMIT_FIELD),
        None => None,
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
