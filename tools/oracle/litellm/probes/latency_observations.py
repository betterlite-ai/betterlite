import asyncio
import warnings
from datetime import UTC, datetime, timedelta, tzinfo
from typing import ClassVar, Final, Literal, Protocol, Self, final
from unittest.mock import patch

import litellm
from litellm import ModelResponse
from litellm.caching.dual_cache import DualCache
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.router_strategy import lowest_latency
from litellm.router_strategy.lowest_latency import LowestLatencyLoggingHandler
from litellm.types.utils import Usage
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

from probes.protocol import Millis, Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true
# pyright: reportMissingTypeStubs=false, reportUnknownMemberType=false, reportUnusedCallResult=false, reportUnreachable=false


class SettingsInput(Strict):
    ttl: float
    lowest_latency_buffer: float
    max_latency_list_size: int
    ttft_percentile: float | None = None


class UsageInput(Strict):
    completion_tokens: int
    total_tokens: int


class Step(Strict):
    kind: Literal["success", "failure", "view", "score"]
    now_millis: Millis
    channels: Literal["metadata", "litellm_metadata", "none"] = "metadata"
    model_group: str | None = None
    metadata_group: str | None = None
    deployment_id: str | None = None
    call_type: str | None = None
    stream: bool = False
    sync: bool = False
    start_millis: Millis = 0
    elapsed_millis: Millis = 0
    first_token_millis: Millis | None = None
    usage: UsageInput | None = None
    failure: Literal["timeout", "other"] = "timeout"
    sample: Literal["latency", "ttft"] = "latency"
    statistic: Literal["mean", "percentile"] = "mean"


class MinuteView(Strict):
    tpm: int
    rpm: int


class DeploymentView(Strict):
    latency: list[float]
    time_to_first_token_seconds: list[float]
    minutes: dict[str, MinuteView]


class StepOutput(Strict):
    kind: Literal["success", "failure", "view", "score"]
    deployments: dict[str, DeploymentView] | None
    score: float | None


class ScenarioOutput(Strict):
    steps: list[StepOutput]


class Cache(Protocol):
    def get_cache(self, key: str) -> object: ...


class Handler(Protocol):
    def log_success_event(
        self, kwargs: dict[str, object], response_obj: ModelResponse, start_time: datetime, end_time: datetime
    ) -> None: ...

    async def async_log_success_event(
        self, kwargs: dict[str, object], response_obj: ModelResponse, start_time: datetime, end_time: datetime
    ) -> None: ...

    async def async_log_failure_event(
        self, kwargs: dict[str, object], response_obj: None, start_time: datetime, end_time: datetime
    ) -> None: ...

    def get_available_deployments(
        self,
        model_group: str,
        healthy_deployments: list[dict[str, JsonValue]],
        *,
        request_kwargs: dict[str, object],
    ) -> dict[str, JsonValue] | None: ...


LATENCY_MAP: Final[TypeAdapter[dict[str, dict[str, JsonValue]]]] = TypeAdapter(dict[str, dict[str, JsonValue]])
MINUTE_USAGE: Final[TypeAdapter[dict[str, int]]] = TypeAdapter(dict[str, int])
SAMPLES: Final[TypeAdapter[list[float]]] = TypeAdapter(list[float])
SCORES: Final[TypeAdapter[dict[str, float]]] = TypeAdapter(dict[str, float])


class StepClock:
    millis: int

    def __init__(self) -> None:
        self.millis = 0

    def wall(self) -> float:
        return self.millis / 1000


class FrozenNow(datetime):
    millis: ClassVar[int] = 0

    @classmethod
    @override
    def now(cls, tz: tzinfo | None = None) -> Self:
        return cls.fromtimestamp(FrozenNow.millis / 1000, tz=UTC if tz is None else tz)


def required(value: str | None, what: str) -> str:
    if value is None:
        raise ProbeError("invalid_request", f"the step needs {what}")
    return value


def instant(millis: int) -> datetime:
    return datetime.fromtimestamp(millis / 1000, tz=UTC)


def routing_args(settings: SettingsInput) -> dict[str, JsonValue]:
    return {
        "ttl": settings.ttl,
        "lowest_latency_buffer": settings.lowest_latency_buffer,
        "max_latency_list_size": settings.max_latency_list_size,
        "ttft_percentile": settings.ttft_percentile,
    }


def event_kwargs(step: Step) -> dict[str, object]:
    params: dict[str, object] = {}
    if step.channels == "metadata":
        params["metadata"] = {"model_group": step.model_group}
    elif step.channels == "litellm_metadata":
        params["litellm_metadata"] = {"model_group": step.model_group}
        params["metadata"] = {"model_group": step.metadata_group}
    if step.deployment_id is not None:
        params["model_info"] = {"id": step.deployment_id}
    kwargs: dict[str, object] = {"litellm_params": params, "stream": step.stream}
    if step.call_type is not None:
        kwargs["call_type"] = step.call_type
    if step.channels == "litellm_metadata":
        kwargs["litellm_metadata"] = {}
    return kwargs


def response(step: Step) -> ModelResponse:
    if step.usage is None:
        return ModelResponse(model="probe")
    usage = Usage(completion_tokens=step.usage.completion_tokens, total_tokens=step.usage.total_tokens)
    return ModelResponse(model="probe", usage=usage)


def failure_exception(kind: Literal["timeout", "other"]) -> BaseException:
    if kind == "timeout":
        return litellm.Timeout("the probe timed out", "probe", "openai")
    return ValueError("the probe failed")


def deployment_view(entry: dict[str, JsonValue]) -> DeploymentView:
    minutes: dict[str, MinuteView] = {}
    for key, value in entry.items():
        if key == "latency" or key == "time_to_first_token_seconds":
            continue
        usage = MINUTE_USAGE.validate_python(value)
        minutes[key] = MinuteView(tpm=usage["tpm"], rpm=usage["rpm"])
    return DeploymentView(
        latency=SAMPLES.validate_python(entry.get("latency", [])),
        time_to_first_token_seconds=SAMPLES.validate_python(entry.get("time_to_first_token_seconds", [])),
        minutes=minutes,
    )


@final
class Scenario:
    def __init__(self, spec: "LatencyObservations") -> None:
        self.spec = spec
        self.clock = StepClock()
        self.cache = DualCache(in_memory_cache=InMemoryCache(clock=self.clock.wall))
        self.store: Cache = self.cache
        self.handler: Handler = LowestLatencyLoggingHandler(
            router_cache=self.cache, routing_args=routing_args(spec.settings)
        )

    async def execute(self) -> ScenarioOutput:
        observations: list[StepOutput] = []
        # LiteLLM reads its minute bucket from the module's datetime, and the case fixes that instant per step
        with patch.object(lowest_latency, "datetime", FrozenNow):
            for step in self.spec.steps:
                self.clock.millis = step.now_millis
                FrozenNow.millis = step.now_millis
                observations.append(await self.apply(step))
        return ScenarioOutput(steps=observations)

    async def apply(self, step: Step) -> StepOutput:
        if step.kind == "success":
            await self.success(step)
            return StepOutput(kind=step.kind, deployments=None, score=None)
        if step.kind == "failure":
            await self.failure(step)
            return StepOutput(kind=step.kind, deployments=None, score=None)
        if step.kind == "view":
            return StepOutput(kind=step.kind, deployments=self.view(step), score=None)
        return StepOutput(kind=step.kind, deployments=None, score=self.score(step))

    async def success(self, step: Step) -> None:
        kwargs = event_kwargs(step)
        start = instant(step.start_millis)
        end = start + timedelta(milliseconds=step.elapsed_millis)
        if step.first_token_millis is None:
            kwargs["completion_start_time"] = end
        else:
            kwargs["completion_start_time"] = start + timedelta(milliseconds=step.first_token_millis)
        if step.sync:
            self.handler.log_success_event(kwargs, response(step), start, end)
        else:
            await self.handler.async_log_success_event(kwargs, response(step), start, end)

    async def failure(self, step: Step) -> None:
        kwargs = event_kwargs(step)
        kwargs["exception"] = failure_exception(step.failure)
        start = instant(step.start_millis)
        await self.handler.async_log_failure_event(kwargs, None, start, start)

    def view(self, step: Step) -> dict[str, DeploymentView]:
        model_group = required(step.model_group, "a model_group")
        stored = self.store.get_cache(key=f"{model_group}_map")
        if stored is None:
            return {}
        entries = LATENCY_MAP.validate_python(stored)
        return {deployment_id: deployment_view(entry) for deployment_id, entry in entries.items()}

    def score(self, step: Step) -> float:
        model_group = required(step.model_group, "a model_group")
        deployment_id = required(step.deployment_id, "a deployment_id")
        if step.sample == "latency" and step.statistic == "percentile":
            raise ProbeError("invalid_request", "LiteLLM takes no percentile of the latency window")
        percentile = self.spec.settings.ttft_percentile
        if step.sample == "ttft" and step.statistic == "mean" and percentile is not None:
            raise ProbeError("invalid_request", "the first-token mean needs no ttft_percentile")
        if step.sample == "ttft" and step.statistic == "percentile" and percentile is None:
            raise ProbeError("invalid_request", "a percentile score needs a configured ttft_percentile")
        metadata: dict[str, object] = {}
        request_kwargs: dict[str, object] = {"metadata": metadata, "stream": step.sample == "ttft"}
        deployment: dict[str, JsonValue] = {
            "model_info": {"id": deployment_id},
            "litellm_params": {"api_base": deployment_id},
        }
        self.handler.get_available_deployments(model_group, [deployment], request_kwargs=request_kwargs)
        if "_latency_per_deployment" not in metadata:
            raise ProbeError("invalid_request", f"{model_group!r} has no latency recorded for {deployment_id!r}")
        scores = SCORES.validate_python(metadata["_latency_per_deployment"])
        if deployment_id not in scores:
            raise ProbeError("invalid_request", f"{deployment_id!r} is not among the scored deployments")
        return scores[deployment_id]


class LatencyObservations(Operation):
    settings: SettingsInput
    steps: list[Step]

    @override
    def run(self) -> ScenarioOutput:
        # LiteLLM's logging models warn while they build; neither side compares warnings
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            return asyncio.run(Scenario(self).execute())
