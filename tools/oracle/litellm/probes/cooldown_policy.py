import asyncio
from collections.abc import Callable
from typing import Final, Literal
from unittest.mock import patch

import litellm
from litellm.constants import INTERNAL_CALL_ORIGIN_METADATA_KEY
from litellm.exceptions import (
    APIConnectionError,
    NotFoundError,
    RateLimitError,
    Timeout,
)
from litellm.integrations.prometheus import PrometheusLogger
from litellm.router import Router
from litellm.router_utils.cooldown_handlers import mark_advisor_orchestration_failure
from litellm.router_utils.router_callbacks.track_deployment_metrics import (
    get_deployment_failures_for_current_minute,
    increment_deployment_successes_for_current_minute,
)
from litellm.types.utils import BACKGROUND_RESPONSE_COST_POLL_CALL_ORIGIN
from pydantic import Field
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

_MESSAGE: Final = "probe failure"
_PROVIDER: Final = "openai"

FailureClassName = Literal["not_found", "timeout", "rate_limit", "api_connection"]

_FACTORIES: dict[FailureClassName, Callable[[], Exception]] = {
    "not_found": lambda: NotFoundError(message=_MESSAGE, model="", llm_provider=_PROVIDER),
    "timeout": lambda: Timeout(message=_MESSAGE, model="", llm_provider=_PROVIDER),
    "rate_limit": lambda: RateLimitError(message=_MESSAGE, model="", llm_provider=_PROVIDER),
    "api_connection": lambda: APIConnectionError(message=_MESSAGE, llm_provider=_PROVIDER, model=""),
}


class RouterInput(Strict):
    allowed_fails: int
    default_allowed_fails: int
    cooldown_time: float
    disable_cooldowns: bool


class DeploymentInput(Strict):
    id: str
    model_name: str
    model: str
    api_base: str | None = None


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


class Event(Strict):
    model_name: str
    model_id: str
    api_base: str
    provider: str
    exception_status: str


class Active(Strict):
    model_id: str
    status_code: str
    cooldown_time: str


class Output(Strict):
    cooled: list[bool]
    events: list[Event]
    active: list[Active]
    failures_for_minute: int


class Recorder:
    def __init__(self) -> None:
        self.events: list[Event] = []


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
    failures: int
    failure: FailureInput
    minute: MinuteInput

    @override
    def run(self) -> Output:
        return asyncio.run(self.scenario())

    async def scenario(self) -> Output:
        router = self.build_router()
        for _ in range(self.minute.successes):
            _ = increment_deployment_successes_for_current_minute(router, self.target)
        recorder = Recorder()
        logger = PrometheusLogger.__new__(PrometheusLogger)
        cooled: list[bool] = []
        with (
            patch.object(litellm, "allowed_fails", self.router.default_allowed_fails),
            patch.object(PrometheusLogger, "increment_deployment_cooled_down", RecordCooldown(recorder)),
            patch.object(PrometheusLogger, "set_deployment_complete_outage"),
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
                cooldown_time=f"{value['cooldown_time']:.6f}",
            )
            for model_id, value in router.cooldown_cache.get_active_cooldowns(
                model_ids=router.get_model_ids(), parent_otel_span=None
            )
        ]
        return Output(
            cooled=cooled,
            events=recorder.events,
            active=active,
            failures_for_minute=get_deployment_failures_for_current_minute(router, self.target),
        )

    def build_router(self) -> Router:
        entries: list[dict[str, object]] = []
        for deployment in self.deployments:
            params: dict[str, object] = {"model": deployment.model, "api_key": "sk-probe"}
            if deployment.api_base is not None:
                params["api_base"] = deployment.api_base
            entries.append(
                {
                    "model_name": deployment.model_name,
                    "litellm_params": params,
                    "model_info": {"id": deployment.id},
                }
            )
        return Router(
            model_list=entries,
            allowed_fails=self.router.allowed_fails,
            cooldown_time=self.router.cooldown_time,
            disable_cooldowns=self.router.disable_cooldowns,
        )

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
            "api_key": "sk-probe",
            "model_info": {"id": self.target},
        }
        if failure.background_response_cost_poll:
            litellm_params["metadata"] = {
                INTERNAL_CALL_ORIGIN_METADATA_KEY: BACKGROUND_RESPONSE_COST_POLL_CALL_ORIGIN
            }
        return {"exception": exception, "litellm_params": litellm_params}

    def target_model(self) -> str:
        for deployment in self.deployments:
            if deployment.id == self.target:
                return deployment.model
        raise ProbeError("invalid_request", f"the case names no deployment {self.target}")
