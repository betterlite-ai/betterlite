import asyncio
import warnings
from datetime import UTC, datetime
from typing import Annotated, ClassVar, Final, Literal, Protocol, final
from unittest.mock import patch

from litellm.caching.dual_cache import DualCache
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.router_strategy import lowest_tpm_rpm
from litellm.router_strategy.lowest_tpm_rpm import LowestTPMLoggingHandler
from pydantic import ConfigDict, Field, TypeAdapter
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

DEFAULT_GROUP: Final = "probe-group"
MINUTE_FORMAT: Final = "%H-%M"
MILLIS_PER_SECOND: Final = 1000

WINDOW: Final = TypeAdapter(dict[str, int])


class Entry(Strict):
    id: str
    tpm: int | None = None
    params_tpm: int | None = None
    info_tpm: int | None = None
    rpm: int | None = None
    params_rpm: int | None = None
    info_rpm: int | None = None


class Step(Strict):
    kind: Literal["success", "async_success", "select", "observe", "gate"]
    now_millis: int = 0
    deployment_id: str | None = None
    total_tokens: int | None = None
    metadata: bool | None = None
    call_type: str | None = None
    input_tokens: int | None = None
    token_count_failure: bool | None = None
    healthy: list[str] = Field(default_factory=list)
    fail_reads: bool | None = None
    fail_writes: bool | None = None


class Recorded(Strict):
    kind: Literal["recorded"] = "recorded"
    tpm: list[tuple[str, int]] | None = None
    rpm: list[tuple[str, int]] | None = None
    tpm_ttl: int | None = None
    rpm_ttl: int | None = None


class Selected(Strict):
    kind: Literal["selected"] = "selected"
    id: str | None = None


class Gated(Strict):
    kind: Literal["gated"] = "gated"


StepOutput = Annotated[Recorded | Selected | Gated, Field(discriminator="kind")]


class Output(Strict):
    steps: list[StepOutput]


class ReturnedInfo(Strict):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")
    id: str


class Returned(Strict):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")
    model_info: ReturnedInfo


RETURNED: Final = TypeAdapter(Returned)


class Handler(Protocol):
    def log_success_event(
        self, kwargs: dict[str, object], response_obj: dict[str, object], start_time: datetime, end_time: datetime, /
    ) -> None: ...

    async def async_log_success_event(
        self, kwargs: dict[str, object], response_obj: dict[str, object], start_time: datetime, end_time: datetime, /
    ) -> None: ...

    def get_available_deployments(
        self,
        model_group: str,
        healthy_deployments: list[dict[str, object]],
        messages: list[dict[str, str]] | None = None,
        request_input: str | list[object] | None = None,
        /,
    ) -> dict[str, object] | None: ...


@final
class FrozenClock:
    def __init__(self, current: datetime) -> None:
        self.current = current

    def now(self) -> datetime:
        return self.current


@final
class InjectedCount:
    def __init__(self, tokens: int | None) -> None:
        self.tokens = tokens

    def __call__(self, *, messages: object, text: object) -> int:
        _ = (messages, text)
        if self.tokens is None:
            raise ValueError("the request cannot be counted")
        return self.tokens


# The in-memory tier of the strategy's cache: it stores the dict objects LiteLLM hands back, the way
# InMemoryCache does, and can fail the way a broken store does.
@final
class RecordingCache(InMemoryCache):
    def __init__(self) -> None:
        super().__init__()
        self.stored: dict[str, dict[str, int]] = {}
        self.written_ttls: dict[str, int] = {}
        self.fail_reads = False
        self.fail_writes = False

    @override
    def get_cache(self, key: str, **kwargs: object) -> object:
        _ = kwargs
        if self.fail_reads:
            raise RuntimeError("the store is unreachable")
        return self.stored.get(key)

    @override
    def set_cache(self, key: str, value: object, **kwargs: object) -> None:
        if self.fail_writes:
            raise RuntimeError("the store is unreachable")
        counts = WINDOW.validate_python(value)
        self.stored[key] = counts
        ttl = kwargs.get("ttl")
        if isinstance(ttl, int):
            self.written_ttls[key] = ttl

    def window(self, key: str) -> list[tuple[str, int]] | None:
        if self.fail_reads:
            return None
        value = self.stored.get(key)
        if value is None:
            return None
        return list(value.items())

    def ttl(self, key: str) -> int | None:
        if self.fail_reads:
            return None
        return self.written_ttls.get(key)


def instant(millis: int) -> datetime:
    # LiteLLM labels the minute with datetime.now(); the probe freezes it at the case's instant in UTC
    return datetime.fromtimestamp(millis / MILLIS_PER_SECOND, tz=UTC)


def entry(group: str, deployment: Entry) -> dict[str, object]:
    params: dict[str, object] = {"model": f"openai/{deployment.id}"}
    info: dict[str, object] = {"id": deployment.id}
    built: dict[str, object] = {"model_name": group, "litellm_params": params, "model_info": info}
    limits = (
        (built, deployment.tpm, deployment.rpm),
        (params, deployment.params_tpm, deployment.params_rpm),
        (info, deployment.info_tpm, deployment.info_rpm),
    )
    for target, tpm, rpm in limits:
        if tpm is not None:
            target["tpm"] = tpm
        if rpm is not None:
            target["rpm"] = rpm
    return built


@final
class Scenario:
    def __init__(self, spec: "UsageRouting") -> None:
        self.spec = spec
        self.memory = RecordingCache()
        self.cache = DualCache(in_memory_cache=self.memory)
        args: dict[str, object] = {} if spec.ttl is None else {"ttl": spec.ttl}
        self.handler: Handler = LowestTPMLoggingHandler(router_cache=self.cache, routing_args=args)
        self.entries: dict[str, dict[str, object]] = {
            deployment.id: entry(spec.group, deployment) for deployment in spec.deployments
        }

    async def run(self) -> Output:
        steps: list[StepOutput] = []
        for step in self.spec.steps:
            at = instant(step.now_millis)
            with patch.object(lowest_tpm_rpm, "datetime", FrozenClock(at)):
                steps.append(await self.run_step(step, at))
        return Output(steps=steps)

    async def run_step(self, step: Step, at: datetime) -> StepOutput:
        match step.kind:
            case "success":
                self.handler.log_success_event(self.kwargs(step), self.response(step), at, at)
                return self.observed(step)
            case "async_success":
                await self.handler.async_log_success_event(self.kwargs(step), self.response(step), at, at)
                return self.observed(step)
            case "select":
                return self.selected(step)
            case "observe":
                return self.observed(step)
            case "gate":
                self.memory.fail_reads = bool(step.fail_reads)
                self.memory.fail_writes = bool(step.fail_writes)
                return Gated()

    def kwargs(self, step: Step) -> dict[str, object]:
        params: dict[str, object] = {}
        if step.metadata is not False:
            params["metadata"] = {"model_group": self.spec.group}
        info: dict[str, object] = {}
        if step.deployment_id is not None:
            info["id"] = step.deployment_id
        params["model_info"] = info
        built: dict[str, object] = {"litellm_params": params}
        if step.call_type is not None:
            built["call_type"] = step.call_type
        return built

    def response(self, step: Step) -> dict[str, object]:
        if step.total_tokens is None:
            return {}
        return {"usage": {"total_tokens": step.total_tokens}}

    def observed(self, step: Step) -> Recorded:
        minute = instant(step.now_millis).strftime(MINUTE_FORMAT)
        tokens_key = f"{self.spec.group}:tpm:{minute}"
        requests_key = f"{self.spec.group}:rpm:{minute}"
        return Recorded(
            tpm=self.memory.window(tokens_key),
            rpm=self.memory.window(requests_key),
            tpm_ttl=self.memory.ttl(tokens_key),
            rpm_ttl=self.memory.ttl(requests_key),
        )

    def selected(self, step: Step) -> Selected:
        pool = self.pool(step.healthy)
        tokens = 0 if step.input_tokens is None else step.input_tokens
        counted = None if step.token_count_failure is True else tokens
        with patch.object(lowest_tpm_rpm, "token_counter", InjectedCount(counted)):
            deployment = self.handler.get_available_deployments(self.spec.group, pool)
        if deployment is None:
            return Selected()
        return Selected(id=RETURNED.validate_python(deployment).model_info.id)

    def pool(self, healthy: list[str]) -> list[dict[str, object]]:
        if not healthy:
            return list(self.entries.values())
        selected: list[dict[str, object]] = []
        for name in healthy:
            found = self.entries.get(name)
            if found is None:
                raise ProbeError("invalid_request", f"{name} is not a deployment of the case")
            selected.append(found)
        return selected


class UsageRouting(Operation):
    group: str = DEFAULT_GROUP
    ttl: int | None = None
    deployments: list[Entry] = Field(default_factory=list)
    steps: list[Step] = Field(default_factory=list)

    @override
    def run(self) -> Output:
        # LiteLLM's logging models can emit pydantic warnings; neither side compares them
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            return asyncio.run(Scenario(self).run())
