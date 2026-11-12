import asyncio
import logging
from collections.abc import Sequence
from typing import Final, TypeAlias, TypedDict, cast

from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.hooks.batch_enqueued_tokens import (
    BatchEnqueuedTokenOutcome,
    BatchEnqueuedTokenOverLimit,
    BatchEnqueuedTokenReservation,
    BatchEnqueuedTokenScope,
    BatchEnqueuedTokenStore,
    resolve_batch_enqueued_token_scopes,
)
from litellm.proxy.utils import InternalUsageCache
from opentelemetry.trace import Span
from typing_extensions import override

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
ScriptStep: TypeAlias = int | list[int] | str
LOGGER_NAME: Final = "LiteLLM Proxy"
SCRIPT_KINDS: Final = ("reserve", "refund", "save", "pop")


class Principal(TypedDict):
    api_key: str | None
    team_id: str | None
    metadata: dict[str, int] | None
    team_metadata: dict[str, int] | None


class ScriptSpec(TypedDict):
    reserve: list[ScriptStep]
    refund: list[ScriptStep]


class MemorySpec(TypedDict):
    seed: dict[str, int]
    read: list[str]


class Input(TypedDict):
    tokens: int
    principal: Principal
    redis: ScriptSpec | None
    memory: MemorySpec


class _Trace:
    def __init__(self) -> None:
        self.events: list[Json] = []


class _ScriptRunner:
    def __init__(self, trace: _Trace, kind: str, steps: list[ScriptStep]) -> None:
        self._trace = trace
        self._kind = kind
        self._steps = steps

    async def __call__(self, keys: Sequence[str], args: Sequence[int]) -> list[int]:
        key = keys[0]
        step = self._steps.pop(0)
        self._trace.events.append({"event": self._kind, "key": key, "args": list(args)})
        match step:
            case str():
                raise RuntimeError(step)
            case int():
                return [step]
            case list():
                return [step[0], step[1]]


class _FakeRedis:
    def __init__(self, trace: _Trace, spec: ScriptSpec) -> None:
        self._trace = trace
        self._queues: list[list[ScriptStep]] = [spec["reserve"], spec["refund"], [], []]
        self._registered = 0

    def async_register_script(self, source: str) -> _ScriptRunner:
        del source
        kind = SCRIPT_KINDS[self._registered]
        steps = self._queues[self._registered]
        self._registered += 1
        return _ScriptRunner(self._trace, kind, steps)


class _FakeDualCache:
    def __init__(self, redis: _FakeRedis | None) -> None:
        self.redis_cache = redis


class _FakeUsageCache:
    def __init__(self, redis: _FakeRedis | None, seed: dict[str, int]) -> None:
        self.dual_cache = _FakeDualCache(redis)
        self.values = dict(seed)

    async def async_get_cache(
        self,
        *,
        key: str,
        litellm_parent_otel_span: Span | None,
        local_only: bool = False,
    ) -> int | None:
        if not local_only:
            raise RuntimeError("the enqueued-token store reads local counters only")
        if litellm_parent_otel_span is not None:
            raise RuntimeError("the enqueued-token counter needs no span")
        return self.values.get(key)

    async def async_set_cache(
        self,
        *,
        key: str,
        value: int,
        litellm_parent_otel_span: Span | None,
        local_only: bool = False,
        ttl: float | None = None,
    ) -> None:
        if not local_only:
            raise RuntimeError("the enqueued-token store writes local counters only")
        if litellm_parent_otel_span is not None:
            raise RuntimeError("the enqueued-token counter needs no span")
        if ttl is None:
            raise RuntimeError("the enqueued-token counter write needs a ttl")
        self.values[key] = value


class _WarningCollector(logging.Handler):
    def __init__(self, trace: _Trace) -> None:
        super().__init__(level=logging.WARNING)
        self._trace = trace

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self._trace.events.append({"event": "warning", "level": record.levelname.lower()})


class _Caller:
    def __init__(self, principal: Principal) -> None:
        self.api_key = principal["api_key"]
        self.team_id = principal["team_id"]
        self.metadata = principal["metadata"]
        self.team_metadata = principal["team_metadata"]


def outcome_projection(outcome: BatchEnqueuedTokenOutcome) -> dict[str, Json]:
    match outcome:
        case BatchEnqueuedTokenReservation():
            return {
                "kind": "reservation",
                "tokens": outcome.tokens,
                "scopes": [
                    {"key": scope.key, "value": scope.value, "limit": scope.limit}
                    for scope in outcome.scopes
                ],
                "backend": outcome.backend,
                "owner_present": outcome.owner != "",
            }
        case BatchEnqueuedTokenOverLimit():
            scope: BatchEnqueuedTokenScope = outcome.scope
            return {
                "kind": "over_limit",
                "scope": {"key": scope.key, "value": scope.value, "limit": scope.limit},
                "enqueued": outcome.enqueued,
            }


def run(input_value: Input) -> Json:
    trace = _Trace()
    redis_spec = input_value["redis"]
    redis = None if redis_spec is None else _FakeRedis(trace, redis_spec)
    fake = _FakeUsageCache(redis, input_value["memory"]["seed"])
    cache = cast(InternalUsageCache, fake)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L378
    store = BatchEnqueuedTokenStore(internal_usage_cache=cache)
    caller = cast(UserAPIKeyAuth, _Caller(input_value["principal"]))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_enqueued_tokens.py#L131
    logger = logging.getLogger(LOGGER_NAME)
    collector = _WarningCollector(trace)
    logger.addHandler(collector)
    try:
        scopes = resolve_batch_enqueued_token_scopes(caller)
        outcome = asyncio.run(
            store.reserve(tokens=input_value["tokens"], scopes=scopes)
        )
    finally:
        logger.removeHandler(collector)
    memory: dict[str, Json] = {
        key: fake.values.get(key) for key in input_value["memory"]["read"]
    }
    return {
        "outcome": outcome_projection(outcome),
        "events": trace.events,
        "memory": memory,
    }
