import asyncio
from collections.abc import Callable, Sequence
from datetime import UTC, datetime
from types import TracebackType
from typing import Protocol, TypedDict

from litellm.caching.redis_batch import request_redis_batch_scope
from litellm.caching.redis_cache import RedisCircuitBreaker
from litellm.proxy.hooks import PROXY_HOOKS
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    RateLimitDescriptor,
    RateLimitResponse,
    RateLimitStatus,
    ReservationWindowIdentity,
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override


class IncrementAmounts(TypedDict):
    requests: int
    tokens: int


class ReplyInput(TypedDict, total=False):
    values: list[int | str]
    failure: str


class CallJson(TypedDict):
    keys: list[str]
    args: list[str]


class ReadJson(TypedDict):
    key: str
    local_only: bool


class WriteJson(TypedDict):
    key: str
    value: int | str
    ttl: int
    local_only: bool


class WindowJson(TypedDict):
    counter_key: str
    window_start: str
    backend: str


class ResponseJson(TypedDict):
    overall_code: str
    statuses: list[RateLimitStatus]
    reservation_windows: list[WindowJson]


class Answer(TypedDict):
    response: ResponseJson
    calls: list[CallJson]
    refunds: list[CallJson]
    reads: list[ReadJson]
    writes: list[WriteJson]


class AtomicCheckInput(TypedDict):
    descriptors: list[RateLimitDescriptor]
    increments: list[IncrementAmounts]
    now: int
    pipelined: bool
    fail_closed: bool
    cache: dict[str, int | str]
    replies: list[ReplyInput]
    refund_replies: list[ReplyInput]


class WindowCache(Protocol):
    async def async_get_cache(
        self, key: str, *, litellm_parent_otel_span: JsonValue, local_only: bool
    ) -> int | str | None: ...

    async def async_set_cache(
        self,
        key: str,
        value: int | str,
        ttl: int,
        *,
        litellm_parent_otel_span: JsonValue,
        local_only: bool,
    ) -> None: ...


class LuaScript(Protocol):
    async def __call__(self, *, keys: Sequence[str], args: Sequence[JsonValue]) -> list[int | str]: ...


class FakeServiceLogger:
    async def async_service_success_hook(self, **_fields: JsonValue) -> None: ...

    async def async_service_failure_hook(self, **_fields: JsonValue) -> None: ...


class FakePipeline:
    def __init__(self, replies: list[list[int | str] | Exception]) -> None:
        self.replies = replies
        self.calls: list[CallJson] = []
        self.raise_on_error = True

    def evalsha(self, _sha: str, numkeys: int, *values: str | int) -> None:
        keys = [str(value) for value in values[:numkeys]]
        args = [str(value) for value in values[numkeys:]]
        self.calls.append(CallJson(keys=keys, args=args))

    async def execute(self, *, raise_on_error: bool = True) -> list[list[int | str] | Exception]:
        self.raise_on_error = raise_on_error
        return self.replies

    async def __aenter__(self) -> "FakePipeline":
        return self

    async def __aexit__(
        self,
        _kind: type[BaseException] | None,
        _value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None: ...


class FakeClient:
    def __init__(self, pipeline: FakePipeline) -> None:
        self.pipeline_obj = pipeline

    def __call__(self) -> "FakeClient":
        return self

    def pipeline(self, *, transaction: bool = False) -> FakePipeline:
        return self.pipeline_obj


class FakeRedisCache:
    def __init__(self, pipeline: FakePipeline) -> None:
        self.namespace: str | None = None
        self.redis_kwargs: dict[str, str] = {}
        self._circuit_breaker = RedisCircuitBreaker(failure_threshold=1, recovery_timeout=60)
        self.service_logger_obj = FakeServiceLogger()
        self.init_async_client = FakeClient(pipeline)

    def check_and_fix_namespace(self, key: str) -> str:
        return key

    def async_register_script(self, _script: str) -> LuaScript:
        return PlannedScript([])


class FakeDualCache:
    def __init__(self, redis_cache: FakeRedisCache | None) -> None:
        self.redis_cache = redis_cache


class RecordingCache(WindowCache):
    def __init__(self, values: dict[str, int | str], pipeline: FakePipeline | None) -> None:
        self.dual_cache = FakeDualCache(None if pipeline is None else FakeRedisCache(pipeline))
        self.values = values
        self.reads: list[ReadJson] = []
        self.writes: list[WriteJson] = []

    @override
    async def async_get_cache(
        self, key: str, *, litellm_parent_otel_span: JsonValue, local_only: bool
    ) -> int | str | None:
        self.reads.append(ReadJson(key=key, local_only=local_only))
        return self.values.get(key)

    @override
    async def async_set_cache(
        self,
        key: str,
        value: int | str,
        ttl: int,
        *,
        litellm_parent_otel_span: JsonValue,
        local_only: bool,
    ) -> None:
        self.writes.append(WriteJson(key=key, value=value, ttl=ttl, local_only=local_only))
        self.values[key] = value


class PlannedScript(LuaScript):
    def __init__(self, plan: list[ReplyInput]) -> None:
        self.plan = plan
        self.calls: list[CallJson] = []

    @override
    async def __call__(self, *, keys: Sequence[str], args: Sequence[JsonValue]) -> list[int | str]:
        index = len(self.calls)
        self.calls.append(CallJson(keys=list(keys), args=[str(arg) for arg in args]))
        if index >= len(self.plan):
            raise RuntimeError("the script plan ran out of replies")
        entry = self.plan[index]
        if "failure" in entry:
            raise ConnectionError(entry["failure"])
        return entry["values"]


class FixedTime:
    def __init__(self, epoch: int) -> None:
        self.epoch = epoch

    def __call__(self) -> datetime:
        return datetime.fromtimestamp(self.epoch, tz=UTC)


class FixedFailClosed:
    def __init__(self, *, enabled: bool) -> None:
        self.enabled = enabled

    def __call__(self) -> bool:
        return self.enabled


class AtomicLimiter(Protocol):
    check_and_increment_by_n_script: LuaScript | None  # ast-grep-ignore: no-long-names
    window_guarded_token_increment_script: LuaScript | None  # ast-grep-ignore: no-long-names

    async def atomic_check_and_increment_by_n(  # ast-grep-ignore: no-long-names
        self, descriptors: list[RateLimitDescriptor], increments: list[IncrementAmounts]
    ) -> RateLimitResponse: ...


def make_limiter(cache: RecordingCache, clock: FixedTime, fail_closed: FixedFailClosed) -> AtomicLimiter:
    constructor = TypeAdapter(Callable[..., AtomicLimiter]).validate_python(
        PROXY_HOOKS["parallel_request_limiter"]
    )
    return constructor(cache, clock, fail_closed_resolver=fail_closed)


def planned_reply(entry: ReplyInput) -> list[int | str] | Exception:
    if "failure" in entry:
        return ConnectionError(entry["failure"])
    return entry["values"]


def window_value(window: ReservationWindowIdentity) -> WindowJson:
    counter_key, window_start, backend = window
    return WindowJson(counter_key=counter_key, window_start=window_start, backend=backend)


def response_value(response: RateLimitResponse) -> ResponseJson:
    reserved = response.get("reservation_windows")
    windows = [] if reserved is None else [window_value(window) for window in sorted(reserved)]
    return ResponseJson(
        overall_code=response["overall_code"],
        statuses=response["statuses"],
        reservation_windows=windows,
    )


async def settle(input_value: AtomicCheckInput) -> Answer:
    pipeline: FakePipeline | None = None
    if input_value["pipelined"]:
        pipeline = FakePipeline([planned_reply(entry) for entry in input_value["replies"]])
    cache = RecordingCache(input_value["cache"], pipeline)
    limiter = make_limiter(
        cache,
        FixedTime(input_value["now"]),
        FixedFailClosed(enabled=input_value["fail_closed"]),
    )
    script = PlannedScript(input_value["replies"])
    refund_script = PlannedScript(input_value["refund_replies"])
    limiter.check_and_increment_by_n_script = script
    limiter.window_guarded_token_increment_script = refund_script
    with request_redis_batch_scope():
        response = await limiter.atomic_check_and_increment_by_n(
            input_value["descriptors"], input_value["increments"]
        )
    calls = script.calls if pipeline is None else pipeline.calls
    return Answer(
        response=response_value(response),
        calls=calls,
        refunds=refund_script.calls,
        reads=cache.reads,
        writes=cache.writes,
    )


def run(input_value: AtomicCheckInput) -> Answer:
    with asyncio.Runner() as runner:
        return runner.run(settle(input_value))
