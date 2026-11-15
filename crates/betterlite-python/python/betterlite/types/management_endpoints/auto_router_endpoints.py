from collections.abc import Mapping, Sequence
from datetime import UTC, datetime
from types import MappingProxyType
from typing import ClassVar, Final, Literal, TypeAlias

from pydantic import BaseModel, ConfigDict, Field, JsonValue

# ast-grep-ignore: no-long-names
DEFAULT_ROUTING_TEST_ROUTER_NAME: Final[str] = "auto_router_routing_test"


class RequestComplexityRouterConfig(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow", arbitrary_types_allowed=True)

    plugins: None = None
    classifier_plugin: None = None


# ast-grep-ignore: no-long-names
class ComplexityRouterConfigValidationRequest(BaseModel):
    complexity_router_config: Mapping[str, JsonValue]
    team_id: str | None = None


# ast-grep-ignore: no-long-names
class ComplexityRouterConfigValidationResponse(BaseModel):
    valid: bool
    error: str | None = None


class AutoRouterAvailabilityRequest(BaseModel):
    team_id: str | None = None
    saved_model_id: str | None = None
    complexity_router_config: Mapping[str, JsonValue] | None = None


class AutoRouterAllowance(BaseModel):
    key: str
    limit: int | None
    remaining: int | None
    used_by_this_router: bool = False
    available: bool = True


class AutoRouterAvailabilityResponse(BaseModel):
    allowances: tuple[AutoRouterAllowance, ...]
    error: str | None = None


class AutoRouterRoutingTestRequest(BaseModel):
    prompt: str | None = None
    messages: Sequence[Mapping[str, JsonValue]] | None = None
    system: str | Sequence[Mapping[str, JsonValue]] | None = None
    tools: Sequence[Mapping[str, JsonValue]] | None = None
    complexity_router_config: RequestComplexityRouterConfig
    saved_model_id: str | None = Field(default=None, min_length=1)
    default_model: str | None = None
    router_name: str = DEFAULT_ROUTING_TEST_ROUTER_NAME  # ast-grep-ignore: no-long-names
    team_id: str | None = None

    def wire_body(self) -> Mapping[str, Sequence[Mapping[str, JsonValue]] | str]:
        body: dict[str, Sequence[Mapping[str, JsonValue]] | str] = {}
        if self.messages is not None:
            body["messages"] = self.messages
        if self.system is not None:
            body["system"] = self.system
        if self.tools is not None:
            body["tools"] = self.tools
        return MappingProxyType(body)


class AutoRouterRoutingTestResponse(BaseModel):
    routed_model: str
    routed_model_configured: bool
    routing_decision: Mapping[str, JsonValue]


class AutoRouterCacheBucket(BaseModel):
    turns: int
    hits: int
    hit_rate_pct: float


class AutoRouterCacheStats(BaseModel):
    coverage_pct: float
    hit_rate_pct: float
    same_model: AutoRouterCacheBucket
    first_visit: AutoRouterCacheBucket
    return_to_tier: AutoRouterCacheBucket
    unordered_turns: int
    return_misses_expired: int
    return_misses_within_ttl: int
    return_misses_unknown: int
    ttl_5m_turns: int
    ttl_1h_turns: int


class AutoRouterBenchmarkTotals(BaseModel):
    sessions: int
    turns: int
    avg_turns_per_session: float
    avg_session_seconds: float
    avg_tokens_per_session: float
    spend: float
    classifier_cost: float | None
    savings_estimated_turns: int
    savings_estimated_actual_spend: float
    # ast-grep-ignore: no-long-names
    savings_estimated_classifier_cost: float | None = None
    saved_spend: float | None
    baseline_spend: float | None
    saved_pct: float | None
    saved_per_session: float | None
    cache: AutoRouterCacheStats


class AutoRouterBenchmarkGroup(AutoRouterBenchmarkTotals):
    router_name: str
    router_type: str
    tier_turns: Mapping[str, int] = Field(default_factory=dict)


class AutoRouterSessionResponse(BaseModel):
    session_id: str
    router_name: str
    router_type: str
    turns: int
    last_model: str
    spend: float
    savings_estimated_turns: int
    savings_estimated_actual_spend: float
    saved_spend: float | None
    baseline_spend: float | None
    # ast-grep-ignore: no-long-names
    savings_estimated_baseline_spend: float | None
    baseline_model: str | None
    baseline_models: Mapping[str, int]


class AutoRouterBenchmarksResponse(BaseModel):
    start_date: str
    end_date: str
    routers_in_scope: int
    totals: AutoRouterBenchmarkTotals
    groups: tuple[AutoRouterBenchmarkGroup, ...]


ShadowEvalStatus: TypeAlias = Literal["running", "completed", "stopped"]

ShadowEvalDirection: TypeAlias = Literal["forward", "reverse"]

ShadowEvalTargetType: TypeAlias = Literal["key", "team", "user"]

# ast-grep-ignore: no-long-names
DEFAULT_SHADOW_EVAL_JUDGE_MODEL: Final[str] = "anthropic/claude-sonnet-5"

SHADOW_EVAL_TURN_VALVE: Final[int] = 10_000

SHADOW_EVAL_MAX_ROUTERS: Final[int] = 4


class StartShadowEvalRequest(BaseModel):
    api_key_ids: tuple[str, ...] = Field(default=(), max_length=100)
    team_ids: tuple[str, ...] = Field(default=(), max_length=100)
    user_ids: tuple[str, ...] = Field(default=(), max_length=100)
    models: tuple[str, ...] = Field(default=(), max_length=100)
    router_name: str | None = None
    router_names: tuple[str, ...] = Field(default=(), max_length=SHADOW_EVAL_MAX_ROUTERS)
    direction: ShadowEvalDirection = "forward"
    baseline_model: str | None = None
    shadow_percentage: float = Field(ge=0.1, le=100.0)
    judge_model: str = DEFAULT_SHADOW_EVAL_JUDGE_MODEL  # ast-grep-ignore: no-long-names
    duration_days: int = Field(default=7, ge=1, le=30)
    max_budget: float = Field(default=10.0, ge=0.01, le=10_000)


class ShadowEvalSlice(BaseModel):
    group: str
    turn_count: int
    real_win_rate_pct: float
    shadow_win_rate_pct: float
    tie_rate_pct: float
    avg_judge_confidence: float
    real_spend: float = 0.0
    shadow_spend: float = 0.0
    cache_hit_turns: int = 0


class ShadowEvalResult(BaseModel):
    by_tier: tuple[ShadowEvalSlice, ...]
    by_current_model: tuple[ShadowEvalSlice, ...]
    by_router: tuple[ShadowEvalSlice, ...] = ()
    overall_shadow_win_rate_pct: float
    overall_tie_rate_pct: float
    sampled_real_spend: float = 0.0
    sampled_shadow_spend: float = 0.0
    not_sampled_count: int | None = None
    unjudgeable_count: int | None = None
    shed_count: int | None = None
    withheld_count: int | None = None


class ShadowEvalJobTargetResponse(BaseModel):
    target_type: ShadowEvalTargetType
    target_id: str
    max_turns: int
    max_budget: float | None = None
    stopped_at: datetime | None = None
    attempt_count: int | None = None
    spend: float | None = None
    verdicts: ShadowEvalSlice | None = None
    target_alias: str | None = None
    key_name: str | None = None

    @property
    def budget_spent(self) -> bool:
        over_spend = (
            self.max_budget is not None
            and self.spend is not None
            and self.spend >= self.max_budget
        )
        return over_spend or (
            self.attempt_count is not None and self.attempt_count >= self.max_turns
        )


class ShadowEvalJobResponse(BaseModel):
    job_id: str
    targets: tuple[ShadowEvalJobTargetResponse, ...] = Field(min_length=1)
    router_names: tuple[str, ...] = Field(min_length=1)
    models: tuple[str, ...] = ()
    direction: ShadowEvalDirection = "forward"
    baseline_model: str | None = None
    judge_model: str
    shadow_percentage: float
    created_at: datetime
    ends_at: datetime
    stopped_by: str | None = None
    judged_count: int | None = None
    error_count: int | None = None
    judge_spend: float | None = None
    last_error: str | None = None
    results: ShadowEvalResult | None = None

    @property
    def router_name(self) -> str:
        return self.router_names[0]

    @property
    def status(self) -> ShadowEvalStatus:
        if self.stopped_by is not None:
            return "stopped"
        ends_at = self.ends_at if self.ends_at.tzinfo else self.ends_at.replace(tzinfo=UTC)
        if datetime.now(UTC) >= ends_at:
            return "completed"
        if all(target.budget_spent for target in self.targets):
            return "completed"
        if all(target.stopped_at is not None for target in self.targets):
            return "stopped"
        return "running"
