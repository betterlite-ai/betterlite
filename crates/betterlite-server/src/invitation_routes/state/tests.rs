#![cfg(test)]

use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_user_lookup::cache::UserCache;
use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_auth::auth_user_lookup::lookup::UserLookupRequest;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::team_repository::decoder::TeamRecord;

use super::{AdminScope, InvitationScope as _};
use crate::management_admins::ports::{AdminUserReads, TeamReadError, TeamReads, UserReadError};

// the proxy admin and the userless caller both leave the ports alone; a read here answers an error the admin check turns into a refusal
struct NoUsers;

#[async_trait]
impl AdminUserReads for NoUsers {
    fn fresh_cache(&self) -> Result<Arc<dyn UserCache>, UserReadError> {
        Err(UserReadError::Lookup(UserLookupError::NoDatabase))
    }

    async fn user_object(
        &self,
        _request: UserLookupRequest<'_>,
        _cache: Arc<dyn UserCache>,
    ) -> Result<Option<LiteLLMUserTable>, UserReadError> {
        panic!("this caller reaches no user read")
    }
}

struct NoTeams;

#[async_trait]
impl TeamReads for NoTeams {
    async fn teams_by_ids(&self, _team_ids: &[String]) -> Result<Vec<TeamRecord>, TeamReadError> {
        panic!("this caller reaches no team read")
    }
}

fn scope() -> AdminScope {
    AdminScope {
        users: Arc::new(NoUsers),
        teams: Arc::new(NoTeams),
    }
}

fn principal(text: &str) -> Principal {
    serde_json::from_str(text).expect("a principal")
}

#[tokio::test]
async fn admin_scope_grants_a_proxy_admin_both_checks() {
    let scope = scope();
    let admin = principal(
        r#"{"principal_type":"human","subject":"admin","auth_method":"api_key","roles":["platform_admin"],"user":{"id":"admin-user"}}"#,
    );
    assert!(scope.has_admin_privileges(&admin).await);
    assert!(scope.can_invite_user(&admin, "alice").await);
}

#[tokio::test]
async fn admin_scope_refuses_a_caller_with_neither_admin_path() {
    let scope = scope();
    let service = principal(
        r#"{"principal_type":"service_account","subject":"service-account","auth_method":"api_key"}"#,
    );
    assert!(!scope.has_admin_privileges(&service).await);
    assert!(!scope.can_invite_user(&service, "alice").await);
}
