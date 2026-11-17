pub mod access_windows;
pub mod chat_stream_fallback;
pub mod cooldown_policy;
pub mod cooldown_state;
pub mod deployment_affinity;
pub mod fallback_attempts;
pub mod generalization_integration;
pub mod model_rate_limits;
pub mod native_stream_fallback;
pub mod prompt_affinity;
pub mod request_scheduler;
pub mod responses_stream_fallback;
pub mod routing_groups;
pub mod routing_plugins;
pub mod tag_routing;

use crate::router_core::candidates::CandidateSet;
use crate::router_core::error::RoutingError;
use crate::router_core::route::RouteContext;

// One stage of a route that another task fills; until it lands the stage reports itself unconfigured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteStage {
    CooldownState,
    CooldownPolicy,
    FallbackAttempts,
    ChatStreamFallback,
    NativeStreamFallback,
    GeneralizationIntegration,
    ModelRateLimits,
    TagRouting,
    AccessWindows,
    RoutingGroups,
    DeploymentAffinity,
    PromptAffinity,
    RequestScheduler,
    RoutingPlugins,
    ResponsesStreamFallback,
}

impl RouteStage {
    #[must_use]
    pub const fn configured(self) -> bool {
        match self {
            Self::CooldownState => cooldown_state::CONFIGURED,
            Self::CooldownPolicy => cooldown_policy::CONFIGURED,
            Self::FallbackAttempts => fallback_attempts::CONFIGURED,
            Self::ChatStreamFallback => chat_stream_fallback::CONFIGURED,
            Self::NativeStreamFallback => native_stream_fallback::CONFIGURED,
            Self::GeneralizationIntegration => generalization_integration::CONFIGURED,
            Self::ModelRateLimits => model_rate_limits::CONFIGURED,
            Self::TagRouting => tag_routing::CONFIGURED,
            Self::AccessWindows => access_windows::CONFIGURED,
            Self::RoutingGroups => routing_groups::CONFIGURED,
            Self::DeploymentAffinity => deployment_affinity::CONFIGURED,
            Self::PromptAffinity => prompt_affinity::CONFIGURED,
            Self::RequestScheduler => request_scheduler::CONFIGURED,
            Self::RoutingPlugins => routing_plugins::CONFIGURED,
            Self::ResponsesStreamFallback => responses_stream_fallback::CONFIGURED,
        }
    }
}

// apply: one stage narrows the pool; each arm calls only the module that owns that stage.
pub async fn apply(
    stage: RouteStage,
    candidates: CandidateSet,
    route: &RouteContext<'_>,
) -> Result<CandidateSet, RoutingError> {
    match stage {
        RouteStage::CooldownState => cooldown_state::apply(candidates, route).await,
        RouteStage::CooldownPolicy => cooldown_policy::apply(candidates, route).await,
        RouteStage::FallbackAttempts => fallback_attempts::apply(candidates, route).await,
        RouteStage::ChatStreamFallback => chat_stream_fallback::apply(candidates, route).await,
        RouteStage::NativeStreamFallback => native_stream_fallback::apply(candidates, route).await,
        RouteStage::GeneralizationIntegration => {
            generalization_integration::apply(candidates, route).await
        }
        RouteStage::ModelRateLimits => model_rate_limits::apply(candidates, route).await,
        RouteStage::TagRouting => tag_routing::apply(candidates, route).await,
        RouteStage::AccessWindows => access_windows::apply(candidates, route).await,
        RouteStage::RoutingGroups => routing_groups::apply(candidates, route).await,
        RouteStage::DeploymentAffinity => deployment_affinity::apply(candidates, route).await,
        RouteStage::PromptAffinity => prompt_affinity::apply(candidates, route).await,
        RouteStage::RequestScheduler => request_scheduler::apply(candidates, route).await,
        RouteStage::RoutingPlugins => routing_plugins::apply(candidates, route).await,
        RouteStage::ResponsesStreamFallback => {
            responses_stream_fallback::apply(candidates, route).await
        }
    }
}

// run: a stage whose owner has not landed yet is skipped, and the pool passes through unchanged.
pub async fn run(
    stage: RouteStage,
    candidates: CandidateSet,
    route: &RouteContext<'_>,
) -> Result<CandidateSet, RoutingError> {
    if !stage.configured() {
        return Ok(candidates);
    }
    apply(stage, candidates, route).await
}
