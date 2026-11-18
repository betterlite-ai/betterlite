import asyncio
from collections.abc import Callable
from datetime import UTC, datetime, timedelta
from typing import Annotated, ClassVar, Final, Literal, Protocol
from unittest.mock import patch

import httpx
import litellm
import litellm.router
from litellm.constants import INTERNAL_CALL_ORIGIN_METADATA_KEY
from litellm.exceptions import (
    APIConnectionError,
    APIError,
    AuthenticationError,
    BadGatewayError,
    BadRequestError,
    InternalServerError,
    NotFoundError,
    RateLimitError,
    ServiceUnavailableError,
    Timeout,
)
from litellm.router import Router
from litellm.router_utils import cooldown_handlers
from litellm.router_utils.cooldown_cache import CooldownCacheValue
from litellm.router_utils.router_callbacks.track_deployment_metrics import (
    get_deployment_failures_for_current_minute,
    get_deployment_successes_for_current_minute,
)
from litellm.types.router import AllowedFailsPolicy
from litellm.types.utils import BACKGROUND_RESPONSE_COST_POLL_CALL_ORIGIN
from pydantic import ConfigDict, Field
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

_MESSAGE: Final = "probe failure"
_PROVIDER: Final = "openai"
_MODEL: Final = "openai/gpt-4o"
_FAILURE_THRESHOLD_PERCENT: Final = 0.5
_FAILURE_THRESHOLD_MINIMUM: Final = 5
_SINGLE_DEPLOYMENT_TRAFFIC: Final = 1000
_CALL_START: Final = datetime(2023, 11, 14, tzinfo=UTC)

FailureClassName = Literal[
    "authentication",
    "not_found",
    "bad_request",
    "timeout",
    "rate_limit",
    "service_unavailable",
    "bad_gateway",
    "internal_server",
    "api",
    "api_connection",
]

_FACTORIES: dict[FailureClassName, Callable[[httpx.Response | None], Exception]] = {
    "authentication": lambda response: AuthenticationError(
        message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL, response=response
    ),
    "not_found": lambda response: NotFoundError(
        message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER, response=response
    ),
    "bad_request": lambda response: BadRequestError(
        message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER, response=response
    ),
    "timeout": lambda response: Timeout(message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER, response=response),
    "rate_limit": lambda response: RateLimitError(
        message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER, response=response
    ),
    "service_unavailable": lambda response: ServiceUnavailableError(
        message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL, response=response
    ),
    "bad_gateway": lambda response: BadGatewayError(
        message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL, response=response
    ),
    "internal_server": lambda response: InternalServerError(
        message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER, response=response
    ),
    "api": lambda _response: APIError(
        status_code=500, message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL
    ),
    "api_connection": lambda _response: APIConnectionError(
        message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL
    ),
}


class Policy(AllowedFailsPolicy):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid", strict=True, frozen=True)


class RouterInput(Strict):
    allowed_fails: int = 3
    default_allowed_fails: int = 3
    cooldown_time: float = 5.0
    disable_cooldowns: bool = False
    allowed_fails_policy: Policy | None = None


class Limits(Strict):
    tpm: int | None = None
    rpm: int | None = None
    itpm: int | None = None
    otpm: int | None = None
    params_tpm: int | None = None
    params_rpm: int | None = None
    params_itpm: int | None = None
    params_otpm: int | None = None
    info_tpm: int | None = None
    info_rpm: int | None = None
    info_itpm: int | None = None
    info_otpm: int | None = None


class Catalog(Strict):
    model_group: str
    model: str
    target_id: str
    group_size: int = 1
    alternatives: int = 0
    limits: Limits = Field(default_factory=Limits)


class Deployment(Strict):
    provider_default: bool = False
    allowed_fails: int | None = None
    params_cooldown_time: float | None = None
    info_cooldown_time: float | None = None


class CallerTimeout(Strict):
    client_side_timeout: bool
    timeout_seconds: str | None
    elapsed_seconds: str | None


class FailureInput(Strict):
    class_name: FailureClassName = Field(alias="class")
    status: int | str
    advisor_orchestration: bool = False
    background_response_cost_poll: bool = False
    caller_timeout: CallerTimeout | None = None
    retry_after: str | None = None


class SuccessStep(Strict):
    kind: Literal["success"]
    call_type: str | None = None
    standard_logging_object: bool = True
    total_tokens: int = 0


class ResponseStep(Strict):
    kind: Literal["response"]
    total_tokens: int


class FailureStep(Strict):
    kind: Literal["failure"]
    failure: FailureInput


class AsyncFailureStep(Strict):
    kind: Literal["async_failure"]
    call_type: str | None = None


Step = Annotated[SuccessStep | ResponseStep | FailureStep | AsyncFailureStep, Field(discriminator="kind")]


class StepOutput(Strict):
    key: str | None
    cooldown: bool | None
    successes: int
    failures: int
    tpm: int | None
    rpm: int | None
    cooldown_time: str | None


class Output(Strict):
    steps: list[StepOutput]


class CooldownView(Protocol):
    async def async_get_active_cooldowns(
        self, model_ids: list[str], parent_otel_span: None
    ) -> list[tuple[str, CooldownCacheValue]]: ...


class ObserversView(Protocol):
    @property
    def cooldown_cache(self) -> CooldownView: ...

    async def get_model_group_usage(self, model_group: str) -> tuple[int | None, int | None]: ...

    async def deployment_callback_on_success(
        self,
        kwargs: dict[str, object],
        completion_response: object,
        start_time: object,
        end_time: object,
    ) -> str | None: ...

    async def increment_deployment_usage_for_response(
        self, response: object, request_kwargs: dict[str, object]
    ) -> None: ...

    def deployment_callback_on_failure(
        self,
        kwargs: dict[str, object],
        completion_response: object,
        start_time: object,
        end_time: object,
    ) -> bool: ...

    async def async_deployment_callback_on_failure(
        self,
        kwargs: dict[str, object],
        completion_response: object,
        start_time: object,
        end_time: object,
    ) -> None: ...


def optional(target: dict[str, object], key: str, value: object | None) -> None:
    if value is not None:
        target[key] = value


def build_exception(failure: FailureInput) -> Exception:
    factory = _FACTORIES.get(failure.class_name)
    if factory is None:
        raise ProbeError("invalid_request", f"the probe cannot build a {failure.class_name} exception")
    response: httpx.Response | None = None
    if failure.retry_after is not None:
        headers = {"retry-after": failure.retry_after}
        request = httpx.Request("GET", "https://probe.invalid")
        response = httpx.Response(int(failure.status), headers=headers, request=request)
    exception = factory(response)
    if failure.advisor_orchestration:
        cooldown_handlers.mark_advisor_orchestration_failure(exception)
    return exception


class FixedClock:
    def __init__(self, now_millis: int) -> None:
        self.now_millis: int = now_millis

    def utc_now(self) -> datetime:
        return datetime.fromtimestamp(self.now_millis / 1000, tz=UTC)


class RouterObservations(Operation):
    now_millis: int
    catalog: Catalog
    deployment: Deployment = Field(default_factory=Deployment)
    router: RouterInput = Field(default_factory=RouterInput)
    steps: list[Step]

    @override
    def run(self) -> Output:
        clock = FixedClock(self.now_millis)
        with (
            patch.object(litellm.router, "get_utc_datetime", clock.utc_now),
            patch.object(litellm, "allowed_fails", self.router.default_allowed_fails),
            patch.object(cooldown_handlers, "DEFAULT_FAILURE_THRESHOLD_PERCENT", _FAILURE_THRESHOLD_PERCENT),
            patch.object(
                cooldown_handlers,
                "DEFAULT_FAILURE_THRESHOLD_MINIMUM_REQUESTS",
                _FAILURE_THRESHOLD_MINIMUM,
            ),
            patch.object(
                cooldown_handlers,
                "SINGLE_DEPLOYMENT_TRAFFIC_FAILURE_THRESHOLD",
                _SINGLE_DEPLOYMENT_TRAFFIC,
            ),
        ):
            return asyncio.run(self.execute())

    async def execute(self) -> Output:
        router = self.build_router()
        metadata = self.metadata()
        outputs: list[StepOutput] = []
        for step in self.steps:
            key, cooldown = await self.run_step(router, metadata, step)
            outputs.append(await self.observe(router, key, cooldown=cooldown))
        return Output(steps=outputs)

    def entry(self, index: int) -> dict[str, object]:
        limits = self.catalog.limits
        identifier = self.catalog.target_id if index == 0 else f"{self.catalog.target_id}-{index}"
        params: dict[str, object] = {"model": self.catalog.model, "api_key": "sk-probe"}
        info: dict[str, object] = {"id": identifier}
        entry: dict[str, object] = {
            "model_name": self.catalog.model_group,
            "litellm_params": params,
            "model_info": info,
        }
        if index == 0:
            optional(params, "tpm", limits.params_tpm)
            optional(params, "rpm", limits.params_rpm)
            optional(params, "itpm", limits.params_itpm)
            optional(params, "otpm", limits.params_otpm)
            optional(info, "tpm", limits.info_tpm)
            optional(info, "rpm", limits.info_rpm)
            optional(info, "itpm", limits.info_itpm)
            optional(info, "otpm", limits.info_otpm)
            optional(params, "cooldown_time", self.deployment.params_cooldown_time)
            optional(info, "cooldown_time", self.deployment.info_cooldown_time)
            optional(info, "allowed_fails", self.deployment.allowed_fails)
            optional(entry, "tpm", limits.tpm)
            optional(entry, "rpm", limits.rpm)
            optional(entry, "itpm", limits.itpm)
            optional(entry, "otpm", limits.otpm)
        return entry

    def build_router(self) -> Router:
        entries = [self.entry(index) for index in range(self.catalog.group_size)]
        alternatives: list[dict[str, object]] = [
            {
                "model_name": f"{self.catalog.model_group}-alternative",
                "litellm_params": {"model": self.catalog.model, "api_key": "sk-probe"},
                "model_info": {
                    "id": f"{self.catalog.target_id}-alternative-{index}",
                    "team_id": "probe-team",
                    "team_public_model_name": "probe-public-model",
                },
            }
            for index in range(self.catalog.alternatives)
        ]
        entries.extend(alternatives)
        router = Router(
            model_list=entries,
            allowed_fails=self.router.allowed_fails,
            allowed_fails_policy=self.router.allowed_fails_policy,
            cooldown_time=self.router.cooldown_time,
            disable_cooldowns=self.router.disable_cooldowns,
        )
        if self.deployment.provider_default:
            router.provider_default_deployment_ids = [self.catalog.target_id]
        return router

    def metadata(self) -> dict[str, object]:
        return {
            "model_group": self.catalog.model_group,
            "deployment": self.catalog.model,
            "model_info": {"id": self.catalog.target_id},
        }

    def request_params(self, metadata: dict[str, object]) -> dict[str, object]:
        info: dict[str, object] = {"id": self.catalog.target_id}
        optional(info, "cooldown_time", self.deployment.info_cooldown_time)
        params: dict[str, object] = {
            "model": self.catalog.model,
            "metadata": metadata,
            "model_info": info,
        }
        optional(params, "cooldown_time", self.deployment.params_cooldown_time)
        return params

    def call_kwargs(self, metadata: dict[str, object]) -> dict[str, object]:
        return {
            "model": self.catalog.model_group,
            "metadata": metadata,
            "litellm_params": self.request_params(metadata),
        }

    async def run_step(
        self, router: ObserversView, metadata: dict[str, object], step: Step
    ) -> tuple[str | None, bool | None]:
        match step:
            case SuccessStep():
                kwargs = self.call_kwargs(metadata)
                kwargs["call_type"] = step.call_type
                if step.standard_logging_object:
                    kwargs["standard_logging_object"] = {"total_tokens": step.total_tokens}
                return await router.deployment_callback_on_success(kwargs, None, None, None), None
            case ResponseStep():
                await router.increment_deployment_usage_for_response(
                    {"usage": {"total_tokens": step.total_tokens}},
                    {"model": self.catalog.model_group, "metadata": metadata},
                )
                return None, None
            case FailureStep():
                return None, router.deployment_callback_on_failure(
                    self.failure_kwargs(metadata, step.failure), None, None, None
                )
            case AsyncFailureStep():
                kwargs = self.call_kwargs(metadata)
                kwargs["call_type"] = step.call_type
                await router.async_deployment_callback_on_failure(kwargs, None, None, None)
                return None, None

    def failure_kwargs(self, metadata: dict[str, object], failure: FailureInput) -> dict[str, object]:
        step_metadata = dict(metadata)
        if failure.background_response_cost_poll:
            step_metadata[INTERNAL_CALL_ORIGIN_METADATA_KEY] = BACKGROUND_RESPONSE_COST_POLL_CALL_ORIGIN
        params = self.request_params(step_metadata)
        kwargs: dict[str, object] = {
            "model": self.catalog.model_group,
            "metadata": step_metadata,
            "litellm_params": params,
            "exception": build_exception(failure),
        }
        timeout = failure.caller_timeout
        if timeout is not None:
            params["client_side_timeout"] = timeout.client_side_timeout
            if timeout.timeout_seconds is not None:
                params["timeout"] = float(timeout.timeout_seconds)
            if timeout.elapsed_seconds is not None:
                kwargs["api_call_start_time"] = _CALL_START
                kwargs["end_time"] = _CALL_START + timedelta(seconds=float(timeout.elapsed_seconds))
        return kwargs

    async def observe(
        self, router: Router, key: str | None, *, cooldown: bool | None
    ) -> StepOutput:
        target = self.catalog.target_id
        tpm, rpm = await router.get_model_group_usage(self.catalog.model_group)
        active = await router.cooldown_cache.async_get_active_cooldowns([target], None)
        return StepOutput(
            key=key,
            cooldown=cooldown,
            successes=get_deployment_successes_for_current_minute(router, target),
            failures=get_deployment_failures_for_current_minute(router, target),
            tpm=tpm,
            rpm=rpm,
            cooldown_time=f"{active[0][1]['cooldown_time']:.6f}" if active else None,
        )
