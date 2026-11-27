use std::collections::BTreeSet;

use betterlite_auth::auth_invalidation::AuthInvalidation;
use betterlite_auth::auth_invalidation::transport::InvalidationTransport as _;
use betterlite_auth::auth_key_lookup::jwt::JwtMappingSource;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::member_removal_identity::duplicates::duplicate_member_indexes;
use betterlite_db::member_removal_identity::identity::MemberIdentity;
use betterlite_logging::prometheus::budget_gauges::gauge::GaugeField;
use betterlite_logging::prometheus_label_projection::prometheus_label_factory::LabelSource;
use betterlite_prometheus::collector_factory::collector::Collector;

use super::audit::DeletedKeyAudit;
use super::caller::RemovalCaller;
use super::error::BulkRemovalError;
use super::removal::{TeamRemoval, TeamRemovalSession};
use crate::team_access::access::{TEAM_OR_ORG_ADMIN, TeamAccess};
use crate::team_membership_effects::metric::emit_team_members_metric;
use crate::team_problems::{forbidden, team_not_found};
use crate::team_request_contracts::bulk_members::TeamMemberDeleteResult;

pub struct BulkTeamRemoval<'deps, Gauge: Collector> {
    pub session: &'deps dyn TeamRemovalSession,
    pub access: &'deps TeamAccess,
    pub jwt: &'deps dyn JwtMappingSource,
    pub audit: &'deps dyn DeletedKeyAudit,
    pub invalidation: &'deps AuthInvalidation,
    pub gauge: Option<&'deps GaugeField<Gauge>>,
    pub labels: LabelSource<'deps>,
}

struct PreparedMembers {
    submitted: Vec<MemberIdentity>,
    kept: Vec<MemberIdentity>,
    original_indexes: Vec<usize>,
    duplicates: BTreeSet<usize>,
}

impl<Gauge: Collector> BulkTeamRemoval<'_, Gauge> {
    pub async fn bulk_remove_team_members(
        &self,
        team_id: &str,
        members: &[MemberIdentity],
        auth: &UserAPIKeyAuth,
    ) -> Result<Vec<TeamMemberDeleteResult>, BulkRemovalError> {
        let caller = RemovalCaller::from_auth(auth)?;
        let Some(team) = self.session.find_team(team_id).await? else {
            return Err(BulkRemovalError::from(team_not_found(team_id)));
        };
        let allowed = self
            .access
            .allows(&caller.team_caller(), &team.fields.base, &TEAM_OR_ORG_ADMIN)
            .await?;
        if !allowed {
            return Err(BulkRemovalError::from(forbidden(forbidden_detail(team_id))));
        }
        let prepared = PreparedMembers::new(members);
        let removal = self
            .session
            .remove_members(team_id, &prepared.kept, &caller.identity(), self.jwt)
            .await?;
        self.apply_post_commit(&removal, &caller).await?;
        Ok(prepared.results(&removal))
    }

    async fn apply_post_commit(
        &self,
        removal: &TeamRemoval,
        caller: &RemovalCaller,
    ) -> Result<(), BulkRemovalError> {
        if !removal.deleted_keys.is_empty() {
            self.audit
                .schedule_deleted(&removal.deleted_keys, &caller.key_event_caller())?;
        }
        self.invalidation
            .delete_cache_key_objects(&removal.deleted_key_tokens())
            .await;
        self.invalidation
            .transport
            .evict_and_broadcast(&removal.jwt_mapping_cache_keys, self.invalidation)
            .await;
        emit_team_members_metric(
            self.gauge,
            self.labels,
            &removal.team.fields.base.team_id,
            removal.team.fields.base.team_alias.as_deref(),
            &removal.team.fields.base.members_with_roles,
        );
        Ok(())
    }
}

impl PreparedMembers {
    // duplicates leave before the transaction opens; the kept members keep their submitted order
    fn new(members: &[MemberIdentity]) -> Self {
        let duplicates = duplicate_member_indexes(members);
        let mut kept = Vec::with_capacity(members.len());
        let mut original_indexes = Vec::with_capacity(members.len());
        for (index, member) in members.iter().enumerate() {
            if duplicates.contains(&index) {
                continue;
            }
            original_indexes.push(index);
            kept.push(member.clone());
        }
        Self {
            submitted: members.to_vec(),
            kept,
            original_indexes,
            duplicates,
        }
    }

    fn results(&self, removal: &TeamRemoval) -> Vec<TeamMemberDeleteResult> {
        let matched: BTreeSet<usize> = removal
            .matched
            .iter()
            .filter_map(|index| self.original_indexes.get(*index).copied())
            .collect();
        self.submitted
            .iter()
            .enumerate()
            .map(|(index, member)| TeamMemberDeleteResult {
                user_id: member.user_id.clone(),
                user_email: member.user_email.clone(),
                success: matched.contains(&index),
                error: error_for(index, &matched, &self.duplicates),
            })
            .collect()
    }
}

fn error_for(
    index: usize,
    matched: &BTreeSet<usize>,
    duplicates: &BTreeSet<usize>,
) -> Option<String> {
    if duplicates.contains(&index) {
        return Some("Duplicate member in request".to_owned());
    }
    if matched.contains(&index) {
        return None;
    }
    Some("User not found in team".to_owned())
}

fn forbidden_detail(team_id: &str) -> String {
    format!(
        "Call not allowed. User not proxy admin OR team admin OR org admin for this team. route='/management/v1/teams/{team_id}/members/bulk_delete'"
    )
}
