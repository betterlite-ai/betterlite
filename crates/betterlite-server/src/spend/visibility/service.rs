use std::collections::BTreeSet;
use std::sync::Arc;

use betterlite_auth::auth_user_lookup::lookup::{UserLookup, UserLookupRequest};
use betterlite_auth::management_routes::key_routes::KeyManagementRoute;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::team::TeamBase;

use super::error::VisibilityError;
use super::owners::{OwnerPair, cold_payload_owner};
use super::query::{SpendLogViewer, payload_query};
use super::roles::is_admin_view_safe;
use super::store::{PayloadRow, VisibilityStore};
use crate::team_access::access::is_team_admin;
use crate::team_access::caller::TeamCaller;
use crate::team_access::roster::member_has_permission;

pub struct SpendLogVisibility {
    pub store: Arc<dyn VisibilityStore>,
    pub users: Arc<UserLookup>,
}

// the row an id lookup resolved to, and the provider request_id cold storage is read under
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPayload {
    pub row: Option<PayloadRow>,
    pub storage_key: String,
}

impl SpendLogVisibility {
    // _assert_user_can_view_request_id: any visible owner pair passes; no row at all is the same 403
    pub async fn assert_can_view_request_id(
        &self,
        caller: &TeamCaller,
        lookup_id: &str,
    ) -> Result<(), VisibilityError> {
        if is_admin_view_safe(caller) {
            return Ok(());
        }
        for owner in self.store.owners(lookup_id).await? {
            if self.can_view_owner(caller, &owner).await? {
                return Ok(());
            }
        }
        Err(VisibilityError::Forbidden {
            request_id: lookup_id.to_owned(),
        })
    }

    // _resolve_spend_log_payload_row and _stored_request_id: the stored request_id wins even when empty
    pub async fn resolve_payload_row(
        &self,
        caller: &TeamCaller,
        lookup_id: &str,
    ) -> Result<ResolvedPayload, VisibilityError> {
        let admin = is_admin_view_safe(caller);
        let team_ids = if admin {
            Vec::new()
        } else {
            self.permitted_teams_or_empty(caller).await
        };
        let viewer = SpendLogViewer {
            user_id: caller.user_id.clone(),
            team_ids,
        };
        let query = payload_query(lookup_id, (!admin).then_some(&viewer));
        let rows = self.store.payload_rows(&query).await?;
        if !admin {
            self.require_fetched_owners(caller, &rows, lookup_id)
                .await?;
        }
        let row = rows.into_iter().next();
        let storage_key = row
            .as_ref()
            .map_or_else(|| lookup_id.to_owned(), |found| found.request_id.clone());
        Ok(ResolvedPayload { row, storage_key })
    }

    // _assert_user_owns_cold_storage_payload: the payload's own recorded owner, since it can outlive its row
    pub async fn assert_owns_cold_payload(
        &self,
        caller: &TeamCaller,
        payload: &JsonData,
        lookup_id: &str,
    ) -> Result<(), VisibilityError> {
        if is_admin_view_safe(caller)
            || self
                .can_view_owner(caller, &cold_payload_owner(payload))
                .await?
        {
            return Ok(());
        }
        Err(VisibilityError::Forbidden {
            request_id: lookup_id.to_owned(),
        })
    }

    // _user_can_view_spend_log_owner: direct ownership needs no list role, and an empty team id is no team
    pub async fn can_view_owner(
        &self,
        caller: &TeamCaller,
        owner: &OwnerPair,
    ) -> Result<bool, VisibilityError> {
        if owner.user.is_some() && owner.user == caller.user_id {
            return Ok(true);
        }
        let Some(team_id) = owner.team_id.as_deref().filter(|id| !id.is_empty()) else {
            return Ok(false);
        };
        let Some(team) = self.store.team(team_id).await? else {
            return Ok(false);
        };
        Ok(may_view_team_logs(caller, &team.fields.base))
    }

    // _get_permitted_team_ids_for_spend_logs: the stored user's teams, never one created on the way
    pub async fn permitted_team_ids(
        &self,
        caller: &TeamCaller,
    ) -> Result<Vec<String>, VisibilityError> {
        let request = UserLookupRequest {
            user_id: caller.user_id.as_deref(),
            sso_user_id: None,
            user_email: None,
            user_id_upsert: false,
            check_db_only: None,
        };
        let user = self
            .users
            .get_user_object(request)
            .await
            .map_err(|error| VisibilityError::Users(Box::new(error)))?;
        let Some(found) = user.filter(|stored| !stored.teams.is_empty()) else {
            return Ok(Vec::new());
        };
        let teams = self.store.teams(&found.teams).await?;
        Ok(teams
            .into_iter()
            .filter(|team| may_view_team_logs(caller, &team.fields.base))
            .map(|team| team.fields.base.team_id)
            .collect())
    }

    // _get_permitted_team_ids_for_spend_logs_or_empty: any failure narrows the caller to their own rows
    pub async fn permitted_teams_or_empty(&self, caller: &TeamCaller) -> Vec<String> {
        match self.permitted_team_ids(caller).await {
            Ok(team_ids) => team_ids,
            Err(error) => {
                tracing::warn!(error = %error, "spend log team scope unavailable, showing own rows only");
                Vec::new()
            }
        }
    }

    // _assert_user_owns_fetched_spend_rows: a row inserted after the precheck is checked again here
    async fn require_fetched_owners(
        &self,
        caller: &TeamCaller,
        rows: &[PayloadRow],
        lookup_id: &str,
    ) -> Result<(), VisibilityError> {
        let owners: BTreeSet<OwnerPair> = rows
            .iter()
            .map(|row| OwnerPair {
                user: row.user.clone(),
                team_id: row.team_id.clone(),
            })
            .collect();
        for owner in &owners {
            if !self.can_view_owner(caller, owner).await? {
                return Err(VisibilityError::Forbidden {
                    request_id: lookup_id.to_owned(),
                });
            }
        }
        Ok(())
    }
}

fn may_view_team_logs(caller: &TeamCaller, team: &TeamBase<String>) -> bool {
    is_team_admin(caller, team)
        || member_has_permission(caller, team, KeyManagementRoute::SpendLogs.value())
}
