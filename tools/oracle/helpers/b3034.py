import asyncio
from collections.abc import Sequence
from datetime import UTC, datetime
from types import TracebackType
from typing import NamedTuple, Protocol, Self, TypedDict, runtime_checkable

from litellm.caching.redis_batch import request_redis_batch_scope
from litellm.caching.redis_cache import RedisCircuitBreaker
from litellm.proxy.hooks import PROXY_HOOKS
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    RateLimitDescriptor,
    RateLimitResponse,
    RateLimitStatus,
)
from pydantic import ConfigDict, JsonValue, TypeAdapter
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


def reply_outcome(entry: ReplyInput) -> list[int | str] | Exception:
    failure = entry.get("failure")
    if failure is not None:
        return ConnectionError(failure)
    values = entry.get("values")
    if values is None:
        raise ValueError("a reply with no failure carries values")
    return values


class PlannedScript:
    def __init__(self, plan: list[ReplyInput]) -> None:
        self.plan = plan
        self.calls: list[CallJson] = []

    async def __call__(
        self, *, keys: Sequence[str], args: Sequence[JsonValue]
    ) -> list[int | str]:
        index = len(self.calls)
        self.calls.append(CallJson(keys=list(keys), args=[str(arg) for arg in args]))
        if index >= len(self.plan):
            raise RuntimeError("the script plan ran out of replies")
        match reply_outcome(self.plan[index]):
            case Exception() as failure:
                raise failure
            case list() as values:
                return values


class FakeServiceLogger:
    async def async_service_success_hook(self, **_fields: JsonValue) -> None: ...

    async def async_service_failure_hook(self, **_fields: JsonValue) -> None: ...


class FakePipeline:
    def __init__(self, replies: list[ReplyInput]) -> None:
        self.replies = replies
        self.calls: list[CallJson] = []
        self.transaction = True
        self.raise_on_error = True

    def evalsha(self, _sha: str, numkeys: int, *values: str | int) -> None:
        keys = [str(value) for value in values[:numkeys]]
        args = [str(value) for value in values[numkeys:]]
        self.calls.append(CallJson(keys=keys, args=args))

    async def execute(self, *, raise_on_error: bool = True) -> list[list[int | str] | Exception]:
        self.raise_on_error = raise_on_error
        return [reply_outcome(entry) for entry in self.replies]

    async def __aenter__(self) -> Self:
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

    def __call__(self) -> Self:
        return self

    def pipeline(self, *, transaction: bool = False) -> FakePipeline:
        self.pipeline_obj.transaction = transaction
        return self.pipeline_obj


class FakeRedisCache:
    def __init__(self, pipeline: FakePipeline) -> None:
        self.namespace: str | None = None
        self.registered_scripts: list[str] = []
        self._circuit_breaker = RedisCircuitBreaker(failure_threshold=1, recovery_timeout=60)
        self.service_logger_obj = FakeServiceLogger()
        self.init_async_client = FakeClient(pipeline)

    def check_and_fix_namespace(self, key: str) -> str:
        prefix = "" if self.namespace is None else self.namespace + ":"
        return key if key.startswith(prefix) else prefix + key

    def async_register_script(self, script: str) -> PlannedScript:
        self.registered_scripts.append(script)
        return PlannedScript([])


class FakeDualCache(NamedTuple):
    redis_cache: FakeRedisCache | None


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
    check_and_increment_by_n_script: PlannedScript | None  # ast-grep-ignore: no-long-names
    window_guarded_token_increment_script: PlannedScript | None  # ast-grep-ignore: no-long-names

    async def atomic_check_and_increment_by_n(  # ast-grep-ignore: no-long-names
        self, descriptors: list[RateLimitDescriptor], increments: list[IncrementAmounts]
    ) -> RateLimitResponse: ...


@runtime_checkable
class LimiterConstructor(Protocol):
    def __call__(
        self, cache: RecordingCache, clock: FixedTime, *, fail_closed_resolver: FixedFailClosed
    ) -> AtomicLimiter: ...


def make_limiter(
    cache: RecordingCache, clock: FixedTime, fail_closed: FixedFailClosed
) -> AtomicLimiter:
    constructor = TypeAdapter[LimiterConstructor](
        LimiterConstructor, config=ConfigDict(arbitrary_types_allowed=True)
    ).validate_python(PROXY_HOOKS["parallel_request_limiter"])
    return constructor(cache, clock, fail_closed_resolver=fail_closed)


def response_value(response: RateLimitResponse) -> ResponseJson:
    reserved = response.get("reservation_windows")
    windows: list[WindowJson] = []
    if reserved is not None:
        windows = [
            WindowJson(counter_key=counter_key, window_start=window_start, backend=backend)
            for counter_key, window_start, backend in sorted(reserved)
        ]
    return ResponseJson(
        overall_code=response["overall_code"],
        statuses=response["statuses"],
        reservation_windows=windows,
    )


async def settle(input_value: AtomicCheckInput) -> Answer:
    pipeline = FakePipeline(input_value["replies"]) if input_value["pipelined"] else None
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
    return Answer(
        response=response_value(response),
        calls=script.calls if pipeline is None else pipeline.calls,
        refunds=refund_script.calls,
        reads=cache.reads,
        writes=cache.writes,
    )


def run(input_value: AtomicCheckInput) -> Answer:
    with asyncio.Runner() as runner:
        return runner.run(settle(input_value))
