from collections.abc import Callable
from datetime import UTC, datetime, timedelta
from typing import Annotated, ClassVar, Final, Literal, final
from unittest.mock import patch

import litellm
from litellm.caching.dual_cache import DualCache
from litellm.constants import INTERNAL_CALL_ORIGIN_METADATA_KEY
from litellm.exceptions import (
    APIConnectionError,
    APIError,
    AuthenticationError,
    BadGatewayError,
    BadRequestError,
    ContentPolicyViolationError,
    ContextWindowExceededError,
    InternalServerError,
    MidStreamFallbackError,
    NotFoundError,
    RateLimitError,
    ServiceUnavailableError,
    Timeout,
    UnsupportedParamsError,
)
from litellm.router import Router
from litellm.router_utils.cooldown_handlers import (
    _should_cooldown_deployment,  # pyright: ignore[reportPrivateUsage]
    _should_run_cooldown_logic,  # pyright: ignore[reportPrivateUsage]
    is_advisor_orchestration_failure,
    is_background_response_cost_poll_not_found,
    is_caller_timeout_408,
    mark_advisor_orchestration_failure,
)
from litellm.router_utils.router_callbacks.track_deployment_metrics import (
    increment_deployment_failures_for_current_minute,
    increment_deployment_successes_for_current_minute,
)
from litellm.types.router import AllowedFailsPolicy, DeploymentTypedDict
from litellm.types.utils import BACKGROUND_RESPONSE_COST_POLL_CALL_ORIGIN
from pydantic import ConfigDict, Field
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

_MESSAGE: Final = "probe failure"
_MODEL: Final = "openai/gpt-4o"
_PROVIDER: Final = "openai"
_MODEL_GROUP: Final = "probe-group"
_CALL_START: Final = datetime(2023, 11, 14, tzinfo=UTC)

FailureClassName = Literal[
    "authentication",
    "not_found",
    "permission_denied",
    "bad_request",
    "unprocessable_entity",
    "timeout",
    "rate_limit",
    "budget_exceeded",
    "context_window_exceeded",
    "content_policy_violation",
    "service_unavailable",
    "bad_gateway",
    "internal_server",
    "api",
    "api_connection",
    "api_response_validation",
    "json_schema_validation",
    "unsupported_params",
    "guardrail_raised",
    "mid_stream_fallback",
    "unclassified",
]

_FACTORIES: dict[FailureClassName, Callable[[], Exception]] = {
    "authentication": lambda: AuthenticationError(message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL),
    "not_found": lambda: NotFoundError(message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER),
    "bad_request": lambda: BadRequestError(message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER),
    "timeout": lambda: Timeout(message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER),
    "rate_limit": lambda: RateLimitError(message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER),
    "context_window_exceeded": lambda: ContextWindowExceededError(
        message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER
    ),
    "content_policy_violation": lambda: ContentPolicyViolationError(
        message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER
    ),
    "service_unavailable": lambda: ServiceUnavailableError(
        message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL
    ),
    "bad_gateway": lambda: BadGatewayError(message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL),
    "internal_server": lambda: InternalServerError(message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER),
    "api": lambda: APIError(status_code=500, message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL),
    "api_connection": lambda: APIConnectionError(message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL),
    "unsupported_params": lambda: UnsupportedParamsError(
        message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER
    ),
    "mid_stream_fallback": lambda: MidStreamFallbackError(message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER),
    "unclassified": lambda: Exception(_MESSAGE),
}


class Policy(AllowedFailsPolicy):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid", strict=True, frozen=True)


class RouterInput(Strict):
    allowed_fails: int
    default_allowed_fails: int
    cooldown_time: float
    disable_cooldowns: bool
    allowed_fails_policy: Policy | None = None


class DeploymentInput(Strict):
    id: str
    model_group_size: Annotated[int, Field(ge=1)]
    has_alternatives: bool
    provider_default: bool
    allowed_fails_policy: Policy | None = None
    allowed_fails: int | None = None
    cooldown_time: float | None = None


class CallerTimeout(Strict):
    client_side_timeout: bool
    timeout_seconds: float | None
    elapsed_seconds: float | None


class FailureInput(Strict):
    class_name: FailureClassName = Field(alias="class")
    status: int | str
    advisor_orchestration: bool
    background_response_cost_poll: bool
    caller_timeout: CallerTimeout | None = None


class MinuteInput(Strict):
    successes: int
    failures: int


class Counter(Strict):
    key: str
    ttl_seconds: float
    fails: int


class Output(Strict):
    cooldown: bool
    counter: Counter | None


@final
class ProbeCache(DualCache):
    def __init__(self, prior_fails: int) -> None:
        super().__init__()
        self.prior_fails = prior_fails
        self.counter_key: str | None = None
        self.counter_ttl = 0.0
        self.counter_fails = 0

    @override
    def increment_cache(
        self,
        key: str,
        value: int,
        local_only: bool = False,  # mirrors DualCache.increment_cache's positional flag
        **kwargs: object,
    ) -> int:
        if local_only:
            return super().increment_cache(key=key, value=value, local_only=True, **kwargs)  # pyright: ignore[reportUnknownMemberType]
        self.counter_key = key
        ttl = kwargs.get("ttl")
        self.counter_ttl = float(ttl) if isinstance(ttl, (int, float)) else 0.0
        self.counter_fails = self.prior_fails + value
        return self.counter_fails

    def recorded(self) -> Counter | None:
        if self.counter_key is None:
            return None
        return Counter(key=self.counter_key, ttl_seconds=self.counter_ttl, fails=self.counter_fails)


def build_exception(name: FailureClassName) -> Exception:
    factory = _FACTORIES.get(name)
    if factory is None:
        raise ProbeError("invalid_request", f"the probe cannot build a {name} exception")
    return factory()


def policy_values(policy: AllowedFailsPolicy) -> dict[str, object]:
    fields: tuple[tuple[str, int | None], ...] = (
        ("ContentPolicyViolationErrorAllowedFails", policy.ContentPolicyViolationErrorAllowedFails),
        ("BadRequestErrorAllowedFails", policy.BadRequestErrorAllowedFails),
        ("AuthenticationErrorAllowedFails", policy.AuthenticationErrorAllowedFails),
        ("TimeoutErrorAllowedFails", policy.TimeoutErrorAllowedFails),
        ("RateLimitErrorAllowedFails", policy.RateLimitErrorAllowedFails),
        ("InternalServerErrorAllowedFails", policy.InternalServerErrorAllowedFails),
        ("ServiceUnavailableErrorAllowedFails", policy.ServiceUnavailableErrorAllowedFails),
        ("BadGatewayErrorAllowedFails", policy.BadGatewayErrorAllowedFails),
        ("NotFoundErrorAllowedFails", policy.NotFoundErrorAllowedFails),
    )
    return {name: value for name, value in fields if value is not None}


def model_info(deployment: DeploymentInput) -> dict[str, object]:
    info: dict[str, object] = {"id": deployment.id}
    if deployment.allowed_fails_policy is not None:
        info["allowed_fails_policy"] = policy_values(deployment.allowed_fails_policy)
    if deployment.allowed_fails is not None:
        info["allowed_fails"] = deployment.allowed_fails
    if deployment.cooldown_time is not None:
        info["cooldown_time"] = deployment.cooldown_time
    return info


class CooldownDecisions(Operation):
    router: RouterInput
    deployment: DeploymentInput | None = None
    failure: FailureInput
    minute: MinuteInput
    allowed_fails_before: int
    time_to_cooldown: float | None = None

    @override
    def run(self) -> Output:
        exception = build_exception(self.failure.class_name)
        if self.failure.advisor_orchestration:
            mark_advisor_orchestration_failure(exception)
        deployment = self.deployment
        router = self.build_router(deployment)
        cache = ProbeCache(self.allowed_fails_before)
        router.cache = cache
        if deployment is not None:
            self.seed_minute(router, deployment.id)
        litellm_params: dict[str, object] = {}
        if self.failure.background_response_cost_poll:
            litellm_params["metadata"] = {
                INTERNAL_CALL_ORIGIN_METADATA_KEY: BACKGROUND_RESPONSE_COST_POLL_CALL_ORIGIN
            }
        details = self.model_call_details()
        status = self.failure.status
        with patch.object(litellm, "allowed_fails", self.router.default_allowed_fails):
            if is_advisor_orchestration_failure(exception):
                return Output(cooldown=False, counter=None)
            if is_background_response_cost_poll_not_found(exception, litellm_params):
                return Output(cooldown=False, counter=None)
            if is_caller_timeout_408(details, status):
                return Output(cooldown=False, counter=None)
            if deployment is None:
                return Output(cooldown=False, counter=None)
            if not _should_run_cooldown_logic(router, deployment.id, status, exception, self.time_to_cooldown):
                return Output(cooldown=False, counter=None)
            cooldown = _should_cooldown_deployment(router, deployment.id, status, exception)
        return Output(cooldown=cooldown, counter=cache.recorded())

    def build_router(self, deployment: DeploymentInput | None) -> Router:
        entries: list[DeploymentTypedDict] = []
        if deployment is not None:
            info = model_info(deployment)
            entries.extend(
                DeploymentTypedDict(
                    model_name=_MODEL_GROUP,
                    litellm_params={"model": _MODEL, "api_key": "sk-probe"},
                    model_info=info if index == 0 else {"id": f"{deployment.id}-{index}"},
                )
                for index in range(deployment.model_group_size)
            )
        router = Router(
            model_list=entries,
            allowed_fails=self.router.allowed_fails,
            allowed_fails_policy=self.router.allowed_fails_policy,
            cooldown_time=self.router.cooldown_time,
            disable_cooldowns=self.router.disable_cooldowns,
        )
        if deployment is not None and deployment.provider_default:
            router.provider_default_deployment_ids = [deployment.id]
        return router

    def seed_minute(self, router: Router, deployment_id: str) -> None:
        for _ in range(self.minute.successes):
            _ = increment_deployment_successes_for_current_minute(router, deployment_id)
        for _ in range(self.minute.failures):
            increment_deployment_failures_for_current_minute(router, deployment_id)

    def model_call_details(self) -> dict[str, object]:
        timeout = self.failure.caller_timeout
        details: dict[str, object] = {}
        if timeout is None:
            return details
        litellm_params: dict[str, object] = {"client_side_timeout": timeout.client_side_timeout}
        if timeout.timeout_seconds is not None:
            litellm_params["timeout"] = timeout.timeout_seconds
        details["litellm_params"] = litellm_params
        if timeout.elapsed_seconds is not None:
            details["api_call_start_time"] = _CALL_START
            details["end_time"] = _CALL_START + timedelta(seconds=timeout.elapsed_seconds)
        return details
