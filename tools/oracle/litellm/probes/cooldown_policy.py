import asyncio
from collections.abc import Callable
from typing import Final, Literal
from unittest.mock import patch

import litellm
from litellm.constants import INTERNAL_CALL_ORIGIN_METADATA_KEY
from litellm.exceptions import (
    APIConnectionError,
    APIError,
    AuthenticationError,
    BadGatewayError,
    BadRequestError,
    InternalServerError,
    NotFoundError,
    PermissionDeniedError,
    RateLimitError,
    ServiceUnavailableError,
    Timeout,
)
from litellm.integrations.prometheus import PrometheusLogger
from litellm.router import Router
from litellm.router_utils.cooldown_handlers import mark_advisor_orchestration_failure
from litellm.router_utils.router_callbacks.track_deployment_metrics import (
    get_deployment_failures_for_current_minute,
    increment_deployment_failures_for_current_minute,
    increment_deployment_successes_for_current_minute,
)
from litellm.types.utils import BACKGROUND_RESPONSE_COST_POLL_CALL_ORIGIN
from pydantic import Field
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

_MESSAGE: Final = "probe failure"
_PROVIDER: Final = "openai"
_API_KEY: Final = "sk-probe"

FailureClassName = Literal[
    "authentication",
    "not_found",
    "bad_request",
    "permission_denied",
    "timeout",
    "rate_limit",
    "service_unavailable",
    "bad_gateway",
    "internal_server",
    "api",
    "api_connection",
]

_FACTORIES: dict[FailureClassName, Callable[[], Exception]] = {
    "authentication": lambda: AuthenticationError(message=_MESSAGE, llm_provider=_PROVIDER, model=""),
    "not_found": lambda: NotFoundError(message=_MESSAGE, model="", llm_provider=_PROVIDER),
    "bad_request": lambda: BadRequestError(message=_MESSAGE, model="", llm_provider=_PROVIDER),
    "permission_denied": lambda: PermissionDeniedError(message=_MESSAGE, model="", llm_provider=_PROVIDER),
    "timeout": lambda: Timeout(message=_MESSAGE, model="", llm_provider=_PROVIDER),
    "rate_limit": lambda: RateLimitError(message=_MESSAGE, model="", llm_provider=_PROVIDER),
    "service_unavailable": lambda: ServiceUnavailableError(message=_MESSAGE, llm_provider=_PROVIDER, model=""),
    "bad_gateway": lambda: BadGatewayError(message=_MESSAGE, llm_provider=_PROVIDER, model=""),
    "internal_server": lambda: InternalServerError(message=_MESSAGE, model="", llm_provider=_PROVIDER),
    "api": lambda: APIError(status_code=500, message=_MESSAGE, llm_provider=_PROVIDER, model=""),
    "api_connection": lambda: APIConnectionError(message=_MESSAGE, llm_provider=_PROVIDER, model=""),
}


class RouterInput(Strict):
    allowed_fails: int
    default_allowed_fails: int
    cooldown_time: float
    disable_cooldowns: bool


class ModelInfoInput(Strict):
    allowed_fails: int | None = None
    cooldown_time: float | None = None


class DeploymentInput(Strict):
    id: str
    model_name: str
    model: str
    api_base: str | None = None
    custom_llm_provider: str | None = None
    cooldown_time: float | None = None
    model_info: ModelInfoInput = ModelInfoInput()


class CallerTimeout(Strict):
    client_side_timeout: bool
    timeout_seconds: float | None
    elapsed_seconds: float | None


class FailureInput(Strict):
    class_name: FailureClassName = Field(alias="class")
    status: int | str
    advisor_orchestration: bool
    background_response_cost_poll: bool
    wrapped_exception: FailureClassName | None = None
    caller_timeout: CallerTimeout | None = None


class MinuteInput(Strict):
    successes: int
    failures: int


class Event(Strict):
    model_name: str
    model_id: str
    api_base: str
    provider: str
    exception_status: str


class Outage(Strict):
    model_name: str
    model_id: str
    api_base: str
    provider: str


class Active(Strict):
    model_id: str
    status_code: str
    timestamp: str
    cooldown_time: str


class Output(Strict):
    cooled: list[bool]
    events: list[Event]
    outages: list[Outage]
    active: list[Active]
    failures_for_minute: int


class Recorder:
    def __init__(self) -> None:
        self.events: list[Event] = []
        self.outages: list[Outage] = []


class RecordOutage:
    def __init__(self, recorder: Recorder) -> None:
        self.recorder = recorder

    def __call__(
        self,
        *,
        litellm_model_name: str,
        model_id: str | None,
        api_base: str | None,
        api_provider: str,
    ) -> None:
        self.recorder.outages.append(
            Outage(
                model_name=litellm_model_name,
                model_id=model_id or "",
                api_base=api_base or "",
                provider=api_provider,
            )
        )


class RecordCooldown:
    def __init__(self, recorder: Recorder) -> None:
        self.recorder = recorder

    def __call__(
        self,
        *,
        litellm_model_name: str,
        model_id: str,
        api_base: str,
        api_provider: str,
        exception_status: str,
    ) -> None:
        self.recorder.events.append(
            Event(
                model_name=litellm_model_name,
                model_id=model_id,
                api_base=api_base,
                provider=api_provider,
                exception_status=exception_status,
            )
        )


class CooldownPolicy(Operation):
    now_millis: int
    router: RouterInput
    deployments: list[DeploymentInput]
    target: str
    requested_model_group: str | None = None
    failures: int
    failure: FailureInput
    exception: str
    minute: MinuteInput

    @override
    def run(self) -> Output:
        return asyncio.run(self.scenario())

    async def scenario(self) -> Output:
        router = self.build_router()
        for _ in range(self.minute.successes):
            _ = increment_deployment_successes_for_current_minute(router, self.target)
        for _ in range(self.minute.failures):
            increment_deployment_failures_for_current_minute(router, self.target)
        recorder = Recorder()
        logger = PrometheusLogger.__new__(PrometheusLogger)
        cooled: list[bool] = []
        with (
            patch.object(litellm, "allowed_fails", self.router.default_allowed_fails),
            patch.object(PrometheusLogger, "set_deployment_complete_outage", RecordOutage(recorder)),
            patch.object(PrometheusLogger, "increment_deployment_cooled_down", RecordCooldown(recorder)),
        ):
            litellm.callbacks.append(logger)
            try:
                for _ in range(self.failures):
                    result: object = router.deployment_callback_on_failure(
                        self.failure_kwargs(), None, None, None
                    )
                    cooled.append(result is True)
                    await asyncio.sleep(0)
            finally:
                litellm.callbacks.remove(logger)
        active = [
            Active(
                model_id=model_id,
                status_code=str(value["status_code"]),
                timestamp=f"{value['timestamp']:.6f}",
                cooldown_time=f"{value['cooldown_time']:.6f}",
            )
            for model_id, value in router.cooldown_cache.get_active_cooldowns(
                model_ids=router.get_model_ids(), parent_otel_span=None
            )
        ]
        return Output(
            cooled=cooled,
            events=recorder.events,
            outages=recorder.outages,
            active=active,
            failures_for_minute=get_deployment_failures_for_current_minute(router, self.target),
        )

    def build_router(self) -> Router:
        entries: list[dict[str, object]] = []
        for deployment in self.deployments:
            entries.append(
                {
                    "model_name": deployment.model_name,
                    "litellm_params": self.litellm_params(deployment),
                    "model_info": self.model_info(deployment),
                }
            )
        return Router(
            model_list=entries,
            allowed_fails=self.router.allowed_fails,
            cooldown_time=self.router.cooldown_time,
            disable_cooldowns=self.router.disable_cooldowns,
        )

    def litellm_params(self, deployment: DeploymentInput) -> dict[str, object]:
        params: dict[str, object] = {"model": deployment.model, "api_key": _API_KEY}
        if deployment.api_base is not None:
            params["api_base"] = deployment.api_base
        if deployment.custom_llm_provider is not None:
            params["custom_llm_provider"] = deployment.custom_llm_provider
        if deployment.cooldown_time is not None:
            params["cooldown_time"] = deployment.cooldown_time
        return params

    def model_info(self, deployment: DeploymentInput) -> dict[str, object]:
        info: dict[str, object] = {"id": deployment.id}
        if deployment.model_info.allowed_fails is not None:
            info["allowed_fails"] = deployment.model_info.allowed_fails
        if deployment.model_info.cooldown_time is not None:
            info["cooldown_time"] = deployment.model_info.cooldown_time
        return info

    def failure_kwargs(self) -> dict[str, object]:
        failure = self.failure
        factory = _FACTORIES.get(failure.class_name)
        if factory is None:
            raise ProbeError("invalid_request", f"the probe cannot build a {failure.class_name} exception")
        exception = factory()
        if failure.advisor_orchestration:
            mark_advisor_orchestration_failure(exception)
        litellm_params: dict[str, object] = {
            "model": self.target_model(),
            "api_key": _API_KEY,
            "model_info": {"id": self.target},
        }
        if failure.background_response_cost_poll:
            litellm_params["metadata"] = {
                INTERNAL_CALL_ORIGIN_METADATA_KEY: BACKGROUND_RESPONSE_COST_POLL_CALL_ORIGIN
            }
        if self.requested_model_group is not None:
            litellm_params["metadata"] = {"model_group": self.requested_model_group}
        return {"exception": exception, "litellm_params": litellm_params}

    def target_model(self) -> str:
        for deployment in self.deployments:
            if deployment.id == self.target:
                return deployment.model
        raise ProbeError("invalid_request", f"the case names no deployment {self.target}")
