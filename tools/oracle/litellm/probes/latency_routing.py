import warnings
from datetime import UTC, datetime
from typing import Final, Protocol, final
from unittest.mock import patch

from litellm.caching.dual_cache import DualCache
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.router_strategy import lowest_latency
from litellm.router_strategy.lowest_latency import LowestLatencyLoggingHandler
from pydantic import Field, JsonValue, TypeAdapter
from typing_extensions import override

from probes.protocol import InjectedRandom, Millis, Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true
# pyright: reportMissingTypeStubs=false, reportUnknownMemberType=false, reportUnusedCallResult=false

MILLIS_PER_SECOND: Final = 1000
PROBE_MODEL: Final = "openai/probe"
SCORES: Final[TypeAdapter[dict[str, float]]] = TypeAdapter(dict[str, float])
MODEL_INFO: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
IDENTIFIER: Final[TypeAdapter[str]] = TypeAdapter(str)


class SettingsInput(Strict):
    lowest_latency_buffer: float
    ttft_percentile: float | None = None


class DeploymentInput(Strict):
    id: str
    tpm: int | None = None
    rpm: int | None = None


class MinuteView(Strict):
    tpm: int
    rpm: int


class StoredInput(Strict):
    id: str
    latency: list[float] = Field(default_factory=list)
    time_to_first_token_seconds: list[float] = Field(default_factory=list)
    minutes: dict[str, MinuteView] = Field(default_factory=dict)


class Output(Strict):
    selected: str | None = None
    valid: list[str] = Field(default_factory=list)
    scores: dict[str, float] | None = None


class Handler(Protocol):
    def get_available_deployments(
        self,
        model_group: str,
        healthy_deployments: list[dict[str, JsonValue]],
        *,
        request_kwargs: dict[str, object],
    ) -> dict[str, JsonValue] | None: ...


@final
class Moment:
    def __init__(self, millis: int) -> None:
        self.millis = millis

    def now(self) -> datetime:
        return datetime.fromtimestamp(self.millis / MILLIS_PER_SECOND, tz=UTC)


@final
class InjectedCount:
    def __init__(self, tokens: int) -> None:
        self.tokens = tokens

    def __call__(self, *, messages: object, text: object) -> int:
        _ = (messages, text)
        return self.tokens


# random.sample is replaced by the stored map's own order, so only the pick's own draw is injected
@final
class InjectedPick:
    def __init__(self, generator: InjectedRandom) -> None:
        self.generator = generator
        self.valid: list[str] = []

    def sample(
        self, population: list[tuple[str, dict[str, JsonValue]]], k: int, /
    ) -> list[tuple[str, dict[str, JsonValue]]]:
        return population[:k]

    def choice(self, seq: list[tuple[dict[str, JsonValue], float]]) -> tuple[dict[str, JsonValue], float]:
        self.valid = sorted(identifier(deployment) for deployment, _latency in seq)
        return seq[int(self.generator.random() * len(seq))]


def identifier(entry: dict[str, JsonValue]) -> str:
    info = MODEL_INFO.validate_python(entry["model_info"])
    return IDENTIFIER.validate_python(info["id"])


def routing_args(settings: SettingsInput) -> dict[str, JsonValue]:
    return {
        "lowest_latency_buffer": settings.lowest_latency_buffer,
        "ttft_percentile": settings.ttft_percentile,
    }


def deployment_entry(entry: DeploymentInput) -> dict[str, JsonValue]:
    params: dict[str, JsonValue] = {"model": PROBE_MODEL, "api_base": entry.id}
    if entry.tpm is not None:
        params["tpm"] = entry.tpm
    if entry.rpm is not None:
        params["rpm"] = entry.rpm
    info: dict[str, JsonValue] = {"id": entry.id}
    return {"model_info": info, "litellm_params": params}


def stored_entry(entry: StoredInput) -> dict[str, JsonValue]:
    latency: list[JsonValue] = [sample for sample in entry.latency]
    ttft: list[JsonValue] = [sample for sample in entry.time_to_first_token_seconds]
    stored: dict[str, JsonValue] = {"latency": latency, "time_to_first_token_seconds": ttft}
    for minute, usage in entry.minutes.items():
        window: dict[str, JsonValue] = {"tpm": usage.tpm, "rpm": usage.rpm}
        stored[minute] = window
    return stored


def select(spec: "LatencyRouting", generator: InjectedRandom) -> Output:
    shim = InjectedPick(generator)
    cache = DualCache(in_memory_cache=InMemoryCache())
    stored: dict[str, JsonValue] = {entry.id: stored_entry(entry) for entry in spec.stored}
    cache.set_cache(key=f"{spec.model_group}_map", value=stored)
    handler: Handler = LowestLatencyLoggingHandler(router_cache=cache, routing_args=routing_args(spec.settings))
    healthy = [deployment_entry(entry) for entry in spec.deployments]
    metadata: dict[str, object] = {}
    kwargs: dict[str, object] = {"metadata": metadata, "stream": spec.stream}
    with (
        patch.object(lowest_latency, "datetime", Moment(spec.now_millis)),
        patch.object(lowest_latency, "random", shim),
        patch.object(lowest_latency, "token_counter", InjectedCount(spec.input_tokens)),
    ):
        picked = handler.get_available_deployments(spec.model_group, healthy, request_kwargs=kwargs)
    selected = None if picked is None else identifier(picked)
    scores = None if selected is None else SCORES.validate_python(metadata["_latency_per_deployment"])
    return Output(selected=selected, valid=sorted(shim.valid), scores=scores)


@final
class LatencyRouting(Operation):
    model_group: str
    settings: SettingsInput
    now_millis: Millis
    stream: bool
    input_tokens: int
    deployments: list[DeploymentInput] = Field(default_factory=list)
    stored: list[StoredInput] = Field(default_factory=list)

    @override
    def run_with(self, generator: InjectedRandom) -> Output:
        # LiteLLM's routing models warn while they build; neither side compares warnings
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            return select(self, generator)
