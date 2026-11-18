import asyncio
import warnings
from dataclasses import dataclass
from datetime import UTC, datetime
from typing import Annotated, ClassVar, Final, Literal, Protocol
from unittest.mock import patch

import litellm
from litellm import ModelResponse, Usage
from litellm.caching.dual_cache import DualCache
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.router_strategy import lowest_cost
from litellm.router_strategy.lowest_cost import LowestCostLoggingHandler
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import override

from probes.protocol import Millis, Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

MILLIS_PER_SECOND: Final = 1000
MINUTE_FORMAT: Final = "%Y-%m-%d-%H-%M"
PROBE_MODEL: Final = "probe"
COUNTS: Final[TypeAdapter[dict[str, dict[str, dict[str, int]]]]] = TypeAdapter(
    dict[str, dict[str, dict[str, int]]]
)


class DeploymentInput(Strict):
    id: str
    model: str
    input_cost_per_token: JsonValue | None = None
    output_cost_per_token: JsonValue | None = None
    params_tpm: JsonValue | None = None
    params_rpm: JsonValue | None = None
    info_tpm: JsonValue | None = None
    info_rpm: JsonValue | None = None
    tpm: JsonValue | None = None
    rpm: JsonValue | None = None


class Step(Strict):
    kind: Literal["success", "success_sync", "select", "state"]
    now_millis: Millis = 0
    deployment_id: str | None = None
    call_type: str | None = None
    total_tokens: int | None = None
    input: str | None = None


class ObservedRecorded(Strict):
    kind: Literal["recorded"] = "recorded"
    step: int


class ObservedSelected(Strict):
    kind: Literal["selected"] = "selected"
    step: int
    deployment_id: str | None


class Found(Strict):
    deployment_id: str
    tpm: int
    rpm: int


class ObservedState(Strict):
    kind: Literal["state"] = "state"
    step: int
    entries: list[Found]


Observation = Annotated[
    ObservedRecorded | ObservedSelected | ObservedState,
    Field(discriminator="kind"),
]


class Output(Strict):
    steps: list[Observation]


class Handler(Protocol):
    def log_success_event(
        self, kwargs: dict[str, object], response_obj: object, start_time: object, end_time: object
    ) -> None: ...

    async def async_log_success_event(
        self, kwargs: dict[str, object], response_obj: object, start_time: object, end_time: object
    ) -> None: ...

    async def async_get_available_deployments(
        self,
        model_group: str,
        healthy_deployments: list[dict[str, object]],
        messages: list[dict[str, str]] | None,
        responses_input: str | None,
        request_kwargs: dict[str, object] | None,
        /,
    ) -> dict[str, object] | None: ...


class Cache(Protocol):
    async def async_get_cache(self, key: str) -> object: ...


class Info(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")
    id: str


class Returned(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")
    model_info: Info


RETURNED: Final = TypeAdapter(Returned)


@dataclass(frozen=True)
class StepDatetime:
    millis: int

    def now(self) -> datetime:
        return datetime.fromtimestamp(self.millis / MILLIS_PER_SECOND, tz=UTC)


def required(value: str | None, what: str) -> str:
    if value is None:
        raise ProbeError("invalid_request", f"the step needs {what}")
    return value


def deployment_dict(deployment: DeploymentInput) -> dict[str, object]:
    params: dict[str, object] = {"model": deployment.model}
    params.update(
        {
            field: value
            for field, value in (
                ("input_cost_per_token", deployment.input_cost_per_token),
                ("output_cost_per_token", deployment.output_cost_per_token),
                ("tpm", deployment.params_tpm),
                ("rpm", deployment.params_rpm),
            )
            if value is not None
        }
    )
    info: dict[str, object] = {"id": deployment.id}
    info.update(
        {
            field: value
            for field, value in (("tpm", deployment.info_tpm), ("rpm", deployment.info_rpm))
            if value is not None
        }
    )
    entry: dict[str, object] = {"model_info": info, "litellm_params": params}
    entry.update(
        {
            field: value
            for field, value in (("tpm", deployment.tpm), ("rpm", deployment.rpm))
            if value is not None
        }
    )
    return entry


def model_response(total_tokens: int | None) -> ModelResponse:
    if total_tokens is None:
        return ModelResponse(model=PROBE_MODEL)
    return ModelResponse(model=PROBE_MODEL, usage=Usage(total_tokens=total_tokens))


class CostRouting(Operation):
    model_group: str
    model_cost: dict[str, dict[str, JsonValue]] = Field(default_factory=dict)
    deployments: list[DeploymentInput]
    steps: list[Step]

    @override
    def run(self) -> Output:
        # pydantic warns about ReadOnly qualifiers while LiteLLM builds its response models; nothing compares them
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            return asyncio.run(run_scenario(self))


async def record(index: int, step: Step, handler: Handler, model_group: str) -> ObservedRecorded:
    kwargs: dict[str, object] = {
        "call_type": step.call_type,
        "litellm_params": {
            "metadata": {"model_group": model_group},
            "model_info": {"id": required(step.deployment_id, "a deployment id")},
        },
    }
    response = model_response(step.total_tokens)
    if step.kind == "success_sync":
        handler.log_success_event(kwargs, response, None, None)
    else:
        await handler.async_log_success_event(kwargs, response, None, None)
    return ObservedRecorded(step=index)


async def selected(index: int, step: Step, handler: Handler, operation: CostRouting) -> ObservedSelected:
    healthy = [deployment_dict(deployment) for deployment in operation.deployments]
    picked = await handler.async_get_available_deployments(
        operation.model_group, healthy, None, step.input, None
    )
    deployment_id = None if picked is None else RETURNED.validate_python(picked).model_info.id
    return ObservedSelected(step=index, deployment_id=deployment_id)


async def state(index: int, step: Step, operation: CostRouting, cache: Cache) -> ObservedState:
    raw = await cache.async_get_cache(key=f"cost_map:{operation.model_group}")
    stored: dict[str, dict[str, dict[str, int]]] = (
        {} if raw is None else COUNTS.validate_python(raw)
    )
    minute = StepDatetime(step.now_millis).now().strftime(MINUTE_FORMAT)
    entries: list[Found] = []
    for deployment_id, minutes in stored.items():
        window: dict[str, int] = minutes.get(minute, {})
        entries.append(
            Found(deployment_id=deployment_id, tpm=window.get("tpm", 0), rpm=window.get("rpm", 0))
        )
    return ObservedState(step=index, entries=entries)


async def run_scenario(operation: CostRouting) -> Output:
    litellm.model_cost = dict(operation.model_cost)
    cache = DualCache(in_memory_cache=InMemoryCache())
    handler: Handler = LowestCostLoggingHandler(router_cache=cache)
    reader: Cache = cache
    outputs: list[Observation] = []
    for index, step in enumerate(operation.steps):
        with patch.object(lowest_cost, "datetime", StepDatetime(step.now_millis)):
            match step.kind:
                case "success" | "success_sync":
                    outputs.append(await record(index, step, handler, operation.model_group))
                case "select":
                    outputs.append(await selected(index, step, handler, operation))
                case "state":
                    outputs.append(await state(index, step, operation, reader))
    return Output(steps=outputs)
