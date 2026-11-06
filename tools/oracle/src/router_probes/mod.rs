macro_rules! not_built_probe {
    ($module:ident) => {
        pub type Input = $crate::router_probes::UnbuiltInput;
        pub type Output = $crate::router_probes::UnbuiltOutput;

        pub(crate) fn run(
            _input: &Input,
            _random: &mut $crate::router_probes::random::InjectedRandom<'_>,
        ) -> Result<$crate::router_probes::ProbeOutput, $crate::router_probes::error::OperationError>
        {
            Err($crate::router_probes::error::OperationError::UnknownOperation {
                message: concat!(stringify!($module), " is not built").to_owned(),
            })
        }
    };
}
pub(crate) use not_built_probe;

macro_rules! probe_operations {
    ($($variant:ident => $module:ident),* $(,)?) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum ProbeOperation {
            $($variant($module::Input),)*
        }

        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum ProbeOutput {
            $($variant($module::Output),)*
        }

        pub(crate) fn dispatch_operation(
            operation: &ProbeOperation,
            random: &mut $crate::router_probes::random::InjectedRandom<'_>,
        ) -> Result<ProbeOutput, $crate::router_probes::error::OperationError> {
            match operation {
                $(ProbeOperation::$variant(input) => $module::run(input, random),)*
            }
        }
    };
}

pub mod access_windows;
pub mod adaptive_classification;
pub mod adaptive_concurrency;
pub mod adaptive_feedback;
pub mod adaptive_persistence;
pub mod adaptive_selection;
pub mod admission_control;
pub mod affinity_claims;
pub mod auto_router_access;
pub mod auto_router_preview;
pub mod auto_router_read_api;
pub mod auto_router_rollups;
pub mod automatic_router_pipeline;
pub mod benchmark_aggregates;
pub mod budget_windows;
pub mod cache_cost_routing;
pub mod capability_classifier;
pub mod capability_forecast;
pub mod catalog_updates;
pub mod chat_stream_fallback;
pub mod classifier_context;
pub mod classifier_message_text;
pub mod classifier_verdict;
pub mod compaction_history;
pub mod complexity_adaptive_placement;
pub mod complexity_config;
pub mod complexity_evaluation;
pub mod complexity_pipeline;
pub mod complexity_placement;
pub mod complexity_placement_gates;
pub mod complexity_scoring;
pub mod complexity_sessions;
pub mod complexity_turn_overrides;
pub mod context_placement;
pub mod cooldown_decisions;
pub mod cooldown_policy;
pub mod cooldown_state;
pub mod cost_routing;
pub mod custom_dimensions;
pub mod deployment_affinity;
pub mod deployment_catalog;
pub mod deployment_clients;
pub mod deployment_identity;
pub mod error;
pub mod fallback_attempts;
pub mod fallback_categories;
pub mod fallback_management;
pub mod fallback_observations;
pub mod fallback_plans;
pub mod feedback_signals;
pub mod io_token_limits;
pub mod io_token_reservations;
pub mod jev_classifier;
pub mod lar1_routing;
pub mod latency_observations;
pub mod latency_routing;
pub mod least_busy;
pub mod llm_classifier;
pub mod member_router_writes;
pub mod model_generalizations;
pub mod model_rate_limits;
pub mod native_stream_fallback;
pub mod passthrough_selection;
pub mod pre_routing_registry;
pub mod prompt_affinity;
pub mod quality_router;
pub mod random;
pub mod redis_failure_policy;
pub mod request_scheduler;
pub mod responses_stream_fallback;
pub mod retry_policy;
pub mod router_compaction;
pub mod router_core;
pub mod router_dependencies;
pub mod router_observations;
pub mod router_preview_validation;
pub mod router_settings;
pub mod router_weights;
pub mod routing_baselines;
pub mod routing_budget_state;
pub mod routing_budgets;
pub mod routing_groups;
pub mod routing_plugins;
pub mod routing_read_batch;
pub mod routing_retries;
pub mod routing_state;
pub mod routing_state_store;
pub mod routing_timeouts;
pub mod routing_value_store;
pub mod semantic_router;
pub mod shadow_eval_lifecycle;
pub mod shadow_eval_models;
pub mod shadow_eval_results;
pub mod shadow_eval_validation;
pub mod shadow_result_aggregation;
pub mod single_candidate;
pub mod static_eligibility;
pub mod strategy_dispatch;
pub mod strategy_registration;
pub mod strategy_router_health;
pub mod tag_matchers;
pub mod tag_routing;
mod tests;
pub mod tier_artifact_predictor;
pub mod tier_definitions;
pub mod two_solver_classifier;
pub mod usage_routing;
pub mod usage_routing_v2;
pub mod usage_v2_accounting;
pub mod wildcard_catalog;

use serde::{Deserialize, Serialize};

use crate::OracleError;
use crate::routing_probe_ipc::{ProbeClock, ProbeFailure, ProbeSample};

// a stub operation takes the empty payload the Python stub's model accepts, and nothing else
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnbuiltInput {
    #[serde(skip)]
    _reserved: (),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnbuiltOutput;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u64")]
pub struct Sample(pub u64);

impl TryFrom<u64> for Sample {
    type Error = OracleError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        let _validated = ProbeSample::try_from(value)?;
        Ok(Self(value))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub clock: ProbeClock,
    pub samples: Vec<Sample>,
    pub operation: ProbeOperation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reply {
    Ok {
        output: ProbeOutput,
        warnings: Vec<String>,
    },
    Error {
        kind: ProbeFailure,
        message: String,
    },
}

probe_operations! {
    SingleCandidate => single_candidate,
    AccessWindows => access_windows,
    AdaptiveClassification => adaptive_classification,
    AdaptiveConcurrency => adaptive_concurrency,
    AdaptiveFeedback => adaptive_feedback,
    AdaptivePersistence => adaptive_persistence,
    AdaptiveSelection => adaptive_selection,
    AdmissionControl => admission_control,
    AffinityClaims => affinity_claims,
    AutomaticRouterPipeline => automatic_router_pipeline,
    AutoRouterAccess => auto_router_access,
    AutoRouterPreview => auto_router_preview,
    AutoRouterReadApi => auto_router_read_api,
    AutoRouterRollups => auto_router_rollups,
    BenchmarkAggregates => benchmark_aggregates,
    BudgetWindows => budget_windows,
    CacheCostRouting => cache_cost_routing,
    CapabilityClassifier => capability_classifier,
    CapabilityForecast => capability_forecast,
    CatalogUpdates => catalog_updates,
    ChatStreamFallback => chat_stream_fallback,
    ClassifierContext => classifier_context,
    ClassifierMessageText => classifier_message_text,
    ClassifierVerdict => classifier_verdict,
    CompactionHistory => compaction_history,
    ComplexityAdaptivePlacement => complexity_adaptive_placement,
    ComplexityConfig => complexity_config,
    ComplexityEvaluation => complexity_evaluation,
    ComplexityPipeline => complexity_pipeline,
    ComplexityPlacement => complexity_placement,
    ComplexityPlacementGates => complexity_placement_gates,
    ComplexityScoring => complexity_scoring,
    ComplexitySessions => complexity_sessions,
    ComplexityTurnOverrides => complexity_turn_overrides,
    ContextPlacement => context_placement,
    CooldownDecisions => cooldown_decisions,
    CooldownPolicy => cooldown_policy,
    CooldownState => cooldown_state,
    CostRouting => cost_routing,
    CustomDimensions => custom_dimensions,
    DeploymentAffinity => deployment_affinity,
    DeploymentCatalog => deployment_catalog,
    DeploymentClients => deployment_clients,
    DeploymentIdentity => deployment_identity,
    FallbackAttempts => fallback_attempts,
    FallbackCategories => fallback_categories,
    FallbackManagement => fallback_management,
    FallbackObservations => fallback_observations,
    FallbackPlans => fallback_plans,
    FeedbackSignals => feedback_signals,
    IoTokenLimits => io_token_limits,
    IoTokenReservations => io_token_reservations,
    JevClassifier => jev_classifier,
    Lar1Routing => lar1_routing,
    LatencyObservations => latency_observations,
    LatencyRouting => latency_routing,
    LeastBusy => least_busy,
    LlmClassifier => llm_classifier,
    MemberRouterWrites => member_router_writes,
    ModelGeneralizations => model_generalizations,
    ModelRateLimits => model_rate_limits,
    NativeStreamFallback => native_stream_fallback,
    PassthroughSelection => passthrough_selection,
    PreRoutingRegistry => pre_routing_registry,
    PromptAffinity => prompt_affinity,
    QualityRouter => quality_router,
    RedisFailurePolicy => redis_failure_policy,
    RequestScheduler => request_scheduler,
    ResponsesStreamFallback => responses_stream_fallback,
    RetryPolicy => retry_policy,
    RouterCompaction => router_compaction,
    RouterCore => router_core,
    RouterDependencies => router_dependencies,
    RouterObservations => router_observations,
    RouterPreviewValidation => router_preview_validation,
    RouterSettings => router_settings,
    RouterWeights => router_weights,
    RoutingBaselines => routing_baselines,
    RoutingBudgets => routing_budgets,
    RoutingBudgetState => routing_budget_state,
    RoutingGroups => routing_groups,
    RoutingPlugins => routing_plugins,
    RoutingReadBatch => routing_read_batch,
    RoutingRetries => routing_retries,
    RoutingState => routing_state,
    RoutingStateStore => routing_state_store,
    RoutingTimeouts => routing_timeouts,
    RoutingValueStore => routing_value_store,
    SemanticRouter => semantic_router,
    ShadowEvalLifecycle => shadow_eval_lifecycle,
    ShadowEvalModels => shadow_eval_models,
    ShadowEvalResults => shadow_eval_results,
    ShadowEvalValidation => shadow_eval_validation,
    ShadowResultAggregation => shadow_result_aggregation,
    StaticEligibility => static_eligibility,
    StrategyDispatch => strategy_dispatch,
    StrategyRegistration => strategy_registration,
    StrategyRouterHealth => strategy_router_health,
    TagMatchers => tag_matchers,
    TagRouting => tag_routing,
    TierArtifactPredictor => tier_artifact_predictor,
    TierDefinitions => tier_definitions,
    TwoSolverClassifier => two_solver_classifier,
    UsageRouting => usage_routing,
    UsageRoutingV2 => usage_routing_v2,
    UsageV2Accounting => usage_v2_accounting,
    WildcardCatalog => wildcard_catalog,
}

#[must_use]
pub fn execute(request: &Request) -> Reply {
    let mut random = random::InjectedRandom::from(request.samples.as_slice());
    match dispatch_operation(&request.operation, &mut random) {
        Ok(output) => Reply::Ok {
            output,
            warnings: Vec::new(),
        },
        Err(error) => Reply::Error {
            kind: error.kind(),
            message: error.to_string(),
        },
    }
}
