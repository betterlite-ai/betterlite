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

use std::future::Future;
use std::pin::Pin;

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

// the pool update a stage module returns, erased so the one match below stays one arm per stage
type StageFuture<'route> =
    Pin<Box<dyn Future<Output = Result<CandidateSet, RoutingError>> + Send + 'route>>;

// apply: one stage narrows the pool; each arm calls only the module that owns that stage.
pub async fn apply(
    stage: RouteStage,
    candidates: CandidateSet,
    route: &RouteContext<'_>,
) -> Result<CandidateSet, RoutingError> {
    let stage_future: StageFuture<'_> = match stage {
        RouteStage::CooldownState => Box::pin(cooldown_state::apply(candidates, route)),
        RouteStage::CooldownPolicy => Box::pin(cooldown_policy::apply(candidates, route)),
        RouteStage::FallbackAttempts => Box::pin(fallback_attempts::apply(candidates, route)),
        RouteStage::ChatStreamFallback => Box::pin(chat_stream_fallback::apply(candidates, route)),
        RouteStage::NativeStreamFallback => {
            Box::pin(native_stream_fallback::apply(candidates, route))
        }
        RouteStage::GeneralizationIntegration => {
            Box::pin(generalization_integration::apply(candidates, route))
        }
        RouteStage::ModelRateLimits => Box::pin(model_rate_limits::apply(candidates, route)),
        RouteStage::TagRouting => Box::pin(tag_routing::apply(candidates, route)),
        RouteStage::AccessWindows => Box::pin(access_windows::apply(candidates, route)),
        RouteStage::RoutingGroups => Box::pin(routing_groups::apply(candidates, route)),
        RouteStage::DeploymentAffinity => Box::pin(deployment_affinity::apply(candidates, route)),
        RouteStage::PromptAffinity => Box::pin(prompt_affinity::apply(candidates, route)),
        RouteStage::RequestScheduler => Box::pin(request_scheduler::apply(candidates, route)),
        RouteStage::RoutingPlugins => Box::pin(routing_plugins::apply(candidates, route)),
        RouteStage::ResponsesStreamFallback => {
            Box::pin(responses_stream_fallback::apply(candidates, route))
        }
    };
    stage_future.await
}

// Every stage's own CONFIGURED flag, keyed by the stage it belongs to.
const STAGE_FLAGS: [(RouteStage, bool); 15] = [
    (RouteStage::CooldownState, cooldown_state::CONFIGURED),
    (RouteStage::CooldownPolicy, cooldown_policy::CONFIGURED),
    (RouteStage::FallbackAttempts, fallback_attempts::CONFIGURED),
    (
        RouteStage::ChatStreamFallback,
        chat_stream_fallback::CONFIGURED,
    ),
    (
        RouteStage::NativeStreamFallback,
        native_stream_fallback::CONFIGURED,
    ),
    (
        RouteStage::GeneralizationIntegration,
        generalization_integration::CONFIGURED,
    ),
    (RouteStage::ModelRateLimits, model_rate_limits::CONFIGURED),
    (RouteStage::TagRouting, tag_routing::CONFIGURED),
    (RouteStage::AccessWindows, access_windows::CONFIGURED),
    (RouteStage::RoutingGroups, routing_groups::CONFIGURED),
    (
        RouteStage::DeploymentAffinity,
        deployment_affinity::CONFIGURED,
    ),
    (RouteStage::PromptAffinity, prompt_affinity::CONFIGURED),
    (RouteStage::RequestScheduler, request_scheduler::CONFIGURED),
    (RouteStage::RoutingPlugins, routing_plugins::CONFIGURED),
    (
        RouteStage::ResponsesStreamFallback,
        responses_stream_fallback::CONFIGURED,
    ),
];

// run: a stage whose owner has not landed yet is skipped, and the pool passes through unchanged.
pub async fn run(
    stage: RouteStage,
    candidates: CandidateSet,
    route: &RouteContext<'_>,
) -> Result<CandidateSet, RoutingError> {
    if !STAGE_FLAGS.contains(&(stage, true)) {
        return Ok(candidates);
    }
    apply(stage, candidates, route).await
}

// A stage no task has built yet: every stage module starts from this and replaces it when its behavior lands.
macro_rules! stage_stub {
    ($entry:literal) => {
        use std::future::{Ready, ready};

        use crate::router_core::candidates::CandidateSet;
        use crate::router_core::error::RoutingError;
        use crate::router_core::route::RouteContext;

        pub(super) const CONFIGURED: bool = false;

        pub(super) fn apply(
            _candidates: CandidateSet,
            _route: &RouteContext<'_>,
        ) -> Ready<Result<CandidateSet, RoutingError>> {
            ready(Err(RoutingError::NotBuilt { entry: $entry }))
        }
    };
}
pub(crate) use stage_stub;
