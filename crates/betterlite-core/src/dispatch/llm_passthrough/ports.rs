use async_trait::async_trait;

use crate::api::passthrough_call::{
    PassthroughCallArgs, PassthroughCallError, PassthroughReply, PassthroughRouterCalls,
};

// The router's blocked check: get_model_list for the model and team, then _are_all_deployments_blocked
pub trait BlockedModels: Send + Sync {
    fn all_blocked(&self, model: &str, team_id: Option<&str>) -> bool;
}

// The call port the plan hands the args to; litellm.allm_passthrough_route needs the logging state and HTTP pool this crate does not hold
#[async_trait]
pub trait PassthroughCallPort: Send + Sync {
    async fn call(
        &self,
        args: PassthroughCallArgs,
    ) -> Result<PassthroughReply, PassthroughCallError>;
}

// B8690's PassthroughRouterCalls has an impl Future return no trait object can hold, so its ports get the boxed form
#[async_trait]
impl<Calls> PassthroughCallPort for Calls
where
    Calls: PassthroughRouterCalls,
{
    async fn call(
        &self,
        args: PassthroughCallArgs,
    ) -> Result<PassthroughReply, PassthroughCallError> {
        self.allm_passthrough_route(args).await
    }
}
