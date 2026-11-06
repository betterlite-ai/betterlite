import random
import sys
import time
from typing import Annotated
from warnings import warn

from pydantic import Field
from typing_extensions import override

from probes.access_windows import AccessWindows
from probes.adaptive_classification import AdaptiveClassification
from probes.adaptive_concurrency import AdaptiveConcurrency
from probes.adaptive_feedback import AdaptiveFeedback
from probes.adaptive_persistence import AdaptivePersistence
from probes.adaptive_selection import AdaptiveSelection
from probes.admission_control import AdmissionControl
from probes.affinity_claims import AffinityClaims
from probes.auto_router_access import AutoRouterAccess
from probes.auto_router_preview import AutoRouterPreview
from probes.auto_router_read_api import AutoRouterReadApi
from probes.auto_router_rollups import AutoRouterRollups
from probes.automatic_router_pipeline import AutomaticRouterPipeline
from probes.benchmark_aggregates import BenchmarkAggregates
from probes.budget_windows import BudgetWindows
from probes.cache_cost_routing import CacheCostRouting
from probes.capability_classifier import CapabilityClassifier
from probes.capability_forecast import CapabilityForecast
from probes.catalog_updates import CatalogUpdates
from probes.chat_stream_fallback import ChatStreamFallback
from probes.classifier_context import ClassifierContext
from probes.classifier_message_text import ClassifierMessageText
from probes.classifier_verdict import ClassifierVerdict
from probes.compaction_history import CompactionHistory
from probes.complexity_adaptive_placement import ComplexityAdaptivePlacement
from probes.complexity_config import ComplexityConfig
from probes.complexity_evaluation import ComplexityEvaluation
from probes.complexity_pipeline import ComplexityPipeline
from probes.complexity_placement import ComplexityPlacement
from probes.complexity_placement_gates import ComplexityPlacementGates
from probes.complexity_scoring import ComplexityScoring
from probes.complexity_sessions import ComplexitySessions
from probes.complexity_turn_overrides import ComplexityTurnOverrides
from probes.context_placement import ContextPlacement
from probes.cooldown_decisions import CooldownDecisions
from probes.cooldown_policy import CooldownPolicy
from probes.cooldown_state import CooldownState
from probes.cost_routing import CostRouting
from probes.custom_dimensions import CustomDimensions
from probes.deployment_affinity import DeploymentAffinity
from probes.deployment_catalog import DeploymentCatalog
from probes.deployment_clients import DeploymentClients
from probes.deployment_identity import DeploymentIdentity
from probes.fallback_attempts import FallbackAttempts
from probes.fallback_categories import FallbackCategories
from probes.fallback_management import FallbackManagement
from probes.fallback_observations import FallbackObservations
from probes.fallback_plans import FallbackPlans
from probes.feedback_signals import FeedbackSignals
from probes.io_token_limits import IoTokenLimits
from probes.io_token_reservations import IoTokenReservations
from probes.jev_classifier import JevClassifier
from probes.lar1_routing import Lar1Routing
from probes.latency_observations import LatencyObservations
from probes.latency_routing import LatencyRouting
from probes.least_busy import LeastBusy
from probes.llm_classifier import LlmClassifier
from probes.member_router_writes import MemberRouterWrites
from probes.model_generalizations import ModelGeneralizations
from probes.model_rate_limits import ModelRateLimits
from probes.native_stream_fallback import NativeStreamFallback
from probes.passthrough_selection import PassthroughSelection
from probes.pre_routing_registry import PreRoutingRegistry
from probes.prompt_affinity import PromptAffinity
from probes.protocol import SAMPLE_LIMIT, Clock, Operation, Strict, serve
from probes.quality_router import QualityRouter
from probes.redis_failure_policy import RedisFailurePolicy
from probes.request_scheduler import RequestScheduler
from probes.responses_stream_fallback import ResponsesStreamFallback
from probes.retry_policy import RetryPolicy
from probes.router_compaction import RouterCompaction
from probes.router_core import RouterCore
from probes.router_dependencies import RouterDependencies
from probes.router_observations import RouterObservations
from probes.router_preview_validation import RouterPreviewValidation
from probes.router_settings import RouterSettings
from probes.router_weights import RouterWeights
from probes.routing_baselines import RoutingBaselines
from probes.routing_budget_state import RoutingBudgetState
from probes.routing_budgets import RoutingBudgets
from probes.routing_groups import RoutingGroups
from probes.routing_plugins import RoutingPlugins
from probes.routing_read_batch import RoutingReadBatch
from probes.routing_retries import RoutingRetries
from probes.routing_state import RoutingState
from probes.routing_state_store import RoutingStateStore
from probes.routing_timeouts import RoutingTimeouts
from probes.routing_value_store import RoutingValueStore
from probes.semantic_router import SemanticRouter
from probes.shadow_eval_lifecycle import ShadowEvalLifecycle
from probes.shadow_eval_models import ShadowEvalModels
from probes.shadow_eval_results import ShadowEvalResults
from probes.shadow_eval_validation import ShadowEvalValidation
from probes.shadow_result_aggregation import ShadowResultAggregation
from probes.single_candidate import SingleCandidate
from probes.static_eligibility import StaticEligibility
from probes.strategy_dispatch import StrategyDispatch
from probes.strategy_registration import StrategyRegistration
from probes.strategy_router_health import StrategyRouterHealth
from probes.tag_matchers import TagMatchers
from probes.tag_routing import TagRouting
from probes.tier_artifact_predictor import TierArtifactPredictor
from probes.tier_definitions import TierDefinitions
from probes.two_solver_classifier import TwoSolverClassifier
from probes.usage_routing import UsageRouting
from probes.usage_routing_v2 import UsageRoutingV2
from probes.usage_v2_accounting import UsageV2Accounting
from probes.wildcard_catalog import WildcardCatalog

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true


class EchoOutput(Strict):
    text: str
    clock: Clock
    samples: list[int]


class Echo(Operation):
    text: str
    draws: Annotated[int, Field(ge=0, lt=1 << 32)]
    warnings: list[str]

    @override
    def run(self) -> EchoOutput:
        for message in self.warnings:
            warn(message, stacklevel=1)
        clock = Clock(unix_millis=round(time.time() * 1000), monotonic_millis=round(time.monotonic() * 1000))
        # Echo reports the injected samples back, so it draws through the patched generator on purpose
        samples = [round(random.random() * SAMPLE_LIMIT) for _ in range(self.draws)]  # noqa: S311
        return EchoOutput(text=self.text, clock=clock, samples=samples)


OPERATIONS: dict[str, type[Operation]] = {
    "access_windows": AccessWindows,
    "adaptive_classification": AdaptiveClassification,
    "adaptive_concurrency": AdaptiveConcurrency,
    "adaptive_feedback": AdaptiveFeedback,
    "adaptive_persistence": AdaptivePersistence,
    "adaptive_selection": AdaptiveSelection,
    "admission_control": AdmissionControl,
    "affinity_claims": AffinityClaims,
    "auto_router_access": AutoRouterAccess,
    "auto_router_preview": AutoRouterPreview,
    "auto_router_read_api": AutoRouterReadApi,
    "auto_router_rollups": AutoRouterRollups,
    "automatic_router_pipeline": AutomaticRouterPipeline,
    "benchmark_aggregates": BenchmarkAggregates,
    "budget_windows": BudgetWindows,
    "cache_cost_routing": CacheCostRouting,
    "capability_classifier": CapabilityClassifier,
    "capability_forecast": CapabilityForecast,
    "catalog_updates": CatalogUpdates,
    "chat_stream_fallback": ChatStreamFallback,
    "classifier_context": ClassifierContext,
    "classifier_message_text": ClassifierMessageText,
    "classifier_verdict": ClassifierVerdict,
    "compaction_history": CompactionHistory,
    "complexity_adaptive_placement": ComplexityAdaptivePlacement,
    "complexity_config": ComplexityConfig,
    "complexity_evaluation": ComplexityEvaluation,
    "complexity_pipeline": ComplexityPipeline,
    "complexity_placement": ComplexityPlacement,
    "complexity_placement_gates": ComplexityPlacementGates,
    "complexity_scoring": ComplexityScoring,
    "complexity_sessions": ComplexitySessions,
    "complexity_turn_overrides": ComplexityTurnOverrides,
    "context_placement": ContextPlacement,
    "cooldown_decisions": CooldownDecisions,
    "cooldown_policy": CooldownPolicy,
    "cooldown_state": CooldownState,
    "cost_routing": CostRouting,
    "custom_dimensions": CustomDimensions,
    "deployment_affinity": DeploymentAffinity,
    "deployment_catalog": DeploymentCatalog,
    "deployment_clients": DeploymentClients,
    "deployment_identity": DeploymentIdentity,
    "echo": Echo,
    "fallback_attempts": FallbackAttempts,
    "fallback_categories": FallbackCategories,
    "fallback_management": FallbackManagement,
    "fallback_observations": FallbackObservations,
    "fallback_plans": FallbackPlans,
    "feedback_signals": FeedbackSignals,
    "io_token_limits": IoTokenLimits,
    "io_token_reservations": IoTokenReservations,
    "jev_classifier": JevClassifier,
    "lar1_routing": Lar1Routing,
    "latency_observations": LatencyObservations,
    "latency_routing": LatencyRouting,
    "least_busy": LeastBusy,
    "llm_classifier": LlmClassifier,
    "member_router_writes": MemberRouterWrites,
    "model_generalizations": ModelGeneralizations,
    "model_rate_limits": ModelRateLimits,
    "native_stream_fallback": NativeStreamFallback,
    "passthrough_selection": PassthroughSelection,
    "pre_routing_registry": PreRoutingRegistry,
    "prompt_affinity": PromptAffinity,
    "quality_router": QualityRouter,
    "redis_failure_policy": RedisFailurePolicy,
    "request_scheduler": RequestScheduler,
    "responses_stream_fallback": ResponsesStreamFallback,
    "retry_policy": RetryPolicy,
    "router_compaction": RouterCompaction,
    "router_core": RouterCore,
    "router_dependencies": RouterDependencies,
    "router_observations": RouterObservations,
    "router_preview_validation": RouterPreviewValidation,
    "router_settings": RouterSettings,
    "router_weights": RouterWeights,
    "routing_baselines": RoutingBaselines,
    "routing_budget_state": RoutingBudgetState,
    "routing_budgets": RoutingBudgets,
    "routing_groups": RoutingGroups,
    "routing_plugins": RoutingPlugins,
    "routing_read_batch": RoutingReadBatch,
    "routing_retries": RoutingRetries,
    "routing_state": RoutingState,
    "routing_state_store": RoutingStateStore,
    "routing_timeouts": RoutingTimeouts,
    "routing_value_store": RoutingValueStore,
    "semantic_router": SemanticRouter,
    "shadow_eval_lifecycle": ShadowEvalLifecycle,
    "shadow_eval_models": ShadowEvalModels,
    "shadow_eval_results": ShadowEvalResults,
    "shadow_eval_validation": ShadowEvalValidation,
    "shadow_result_aggregation": ShadowResultAggregation,
    "single_candidate": SingleCandidate,
    "static_eligibility": StaticEligibility,
    "strategy_dispatch": StrategyDispatch,
    "strategy_registration": StrategyRegistration,
    "strategy_router_health": StrategyRouterHealth,
    "tag_matchers": TagMatchers,
    "tag_routing": TagRouting,
    "tier_artifact_predictor": TierArtifactPredictor,
    "tier_definitions": TierDefinitions,
    "two_solver_classifier": TwoSolverClassifier,
    "usage_routing": UsageRouting,
    "usage_routing_v2": UsageRoutingV2,
    "usage_v2_accounting": UsageV2Accounting,
    "wildcard_catalog": WildcardCatalog,
}

if __name__ == "__main__":
    serve(sys.stdin.buffer, sys.stdout, OPERATIONS)
