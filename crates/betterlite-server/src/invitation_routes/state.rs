use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_error::error::ErrorClass;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_router::request_scheduler::service::RequestScheduler;
use betterlite_router::router_core::headers;
use betterlite_router::routing_timeouts::error::TimeoutError;
use betterlite_router::routing_timeouts::global::GlobalRequestTimeout;
use betterlite_router::routing_timeouts::plan::{
    DeploymentTimeouts, RequestTimeouts, RouterTimeoutSources, RouterTimeouts, TimeoutPlan,
    resolve_router_timeouts,
};
use betterlite_router::routing_timeouts::seconds::DEFAULT_REQUEST_TIMEOUT;

use super::store::InvitationStore;
use crate::keepalive_interval::PingSetting;
use crate::management_admins::ports::{AdminUserReads, TeamReads};
use crate::management_admins::privileges::{admin_can_invite_user, user_has_admin_privileges};
use crate::management_admins::scope::AdminCaller;
use crate::pre_call::rewrites::CliOverrides;
use crate::sse_data_generator::types::StreamSource;

mod tests;

// The two access questions /invitation/new and /invitation/delete ask of B863
#[async_trait]
pub trait InvitationScope: Send + Sync {
    async fn has_admin_privileges(&self, caller: &Principal) -> bool;

    async fn can_invite_user(&self, caller: &Principal, target_user_id: &str) -> bool;
}

// B863's admin helpers over the user and team reads the proxy already holds
pub struct AdminScope {
    pub users: Arc<dyn AdminUserReads>,
    pub teams: Arc<dyn TeamReads>,
}

#[async_trait]
impl InvitationScope for AdminScope {
    async fn has_admin_privileges(&self, caller: &Principal) -> bool {
        let admin = AdminCaller::from_principal(caller);
        user_has_admin_privileges(&admin, self.users.as_ref(), Some(self.teams.as_ref()), None)
            .await
    }

    async fn can_invite_user(&self, caller: &Principal, target_user_id: &str) -> bool {
        let admin = AdminCaller::from_principal(caller);
        admin_can_invite_user(
            target_user_id,
            &admin,
            self.users.as_ref(),
            Some(self.teams.as_ref()),
            None,
        )
        .await
    }
}

// The rows the four invitation routes read and write, and the admin checks beside them
pub struct InvitationDatabase {
    pub invitations: Arc<dyn InvitationStore>,
    pub scope: Arc<dyn InvitationScope>,
}

pub struct QueueAnswer {
    pub response: headers::Response,
    pub stream: Option<StreamSource>,
}

// Router.acompletion behind the scheduler: the health count the poll reads, then the call itself
#[async_trait]
pub trait QueueCompletion: Send + Sync {
    fn healthy_deployments(&self, model: &str) -> usize;

    async fn complete(
        &self,
        caller: &Principal,
        data: &JsonMap,
        stream: bool,
    ) -> Result<QueueAnswer, Box<ErrorClass>>;
}

// proxy_logging_obj.post_call_failure_hook, which may replace the exception it is given
#[async_trait]
pub trait QueueFailureHook: Send + Sync {
    async fn post_call_failure(
        &self,
        caller: &Principal,
        exception: &ProxyException,
        request_data: &JsonMap,
    ) -> Option<ProxyException>;
}

// llm_router, which is one object to LiteLLM and two ports here
pub struct QueueRouter {
    pub scheduler: Arc<RequestScheduler>,
    pub completion: Arc<dyn QueueCompletion>,
}

pub struct QueueRouteState {
    pub router: Option<QueueRouter>,
    pub keepalive: Option<PingSetting>,
    pub failure: Option<Arc<dyn QueueFailureHook>>,
    pub completion_model: Option<String>,
    pub cli_model: Option<String>,
    pub overrides: CliOverrides,
    pub timeouts: RouterTimeouts,
}

impl Default for QueueRouteState {
    fn default() -> Self {
        Self {
            router: None,
            keepalive: None,
            failure: None,
            completion_model: None,
            cli_model: None,
            overrides: CliOverrides::default(),
            timeouts: resolve_router_timeouts(
                &RouterTimeoutSources::default(),
                GlobalRequestTimeout {
                    seconds: DEFAULT_REQUEST_TIMEOUT,
                    explicitly_set: false,
                },
            ),
        }
    }
}

#[derive(Default)]
pub struct InvitationRouteState {
    pub invitations: Option<InvitationDatabase>,
    pub queue: QueueRouteState,
}

pub fn timeout_plan(timeouts: &RouterTimeouts, stream: bool) -> Result<TimeoutPlan, TimeoutError> {
    let request = RequestTimeouts {
        stream,
        timeout: None,
        request_timeout: None,
        stream_timeout: None,
    };
    TimeoutPlan::new(
        Instant::now(),
        timeouts,
        &request,
        &DeploymentTimeouts::default(),
    )
}
