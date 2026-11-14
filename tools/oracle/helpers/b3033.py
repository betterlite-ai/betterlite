import asyncio
from collections.abc import Callable, Sequence
from datetime import UTC, datetime
from typing import Literal, NotRequired, Protocol, TypedDict, runtime_checkable

from litellm.proxy.hooks import PROXY_HOOKS
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

LimitType = Literal["requests", "tokens"]


class DescriptorRateLimit(TypedDict, total=False):
    requests_per_unit: int | None
    tokens_per_unit: int | None
    max_parallel_requests: int | None
    window_size: int | None


class Descriptor(TypedDict):
    key: str
    value: str
    rate_limit: DescriptorRateLimit | None


class IncrementAmounts(TypedDict):
    requests: int
    tokens: int


class CounterMeta(TypedDict):
    descriptor_key: str
    descriptor_value: str
    current_limit: int
    rate_limit_type: LimitType
    window_key: str
    counter_key: str
    increment: int
    ttl: int
    window_size: int


class StatusOut(TypedDict):
    code: str
    current_limit: int
    limit_remaining: int
    rate_limit_type: LimitType
    descriptor_key: str
    descriptor_value: str


ReservationWindow = tuple[str, str, Literal["redis", "local"]]


class ResponseOut(TypedDict):
    overall_code: str
    statuses: list[StatusOut]
    reservation_windows: NotRequired[frozenset[ReservationWindow]]


class StatusJson(TypedDict):
    code: str
    current_limit: int
    limit_remaining: int
    rate_limit_type: str
    descriptor_key: str
    descriptor_value: str


class WindowJson(TypedDict):
    counter_key: str
    window_start: str
    backend: str


class ResponseJson(TypedDict):
    overall_code: str
    statuses: list[StatusJson]
    reservation_windows: list[WindowJson]


class CallJson(TypedDict):
    keys: list[str]
    args: list[int]


class ReadJson(TypedDict):
    key: str
    local_only: bool


class WriteJson(TypedDict):
    key: str
    value: int | str
    ttl: int
    local_only: bool


class Answer(TypedDict):
    response: ResponseJson
    calls: list[CallJson]
    reads: list[ReadJson]
    writes: list[WriteJson]


class AtomicCheckInput(TypedDict):
    descriptor: Descriptor
    increment: IncrementAmounts
    default_window_size: int
    now: int
    cache: dict[str, int | str]
    reply: list[int | str] | None


@runtime_checkable
class DualCacheHolder(Protocol):
    redis_cache: JsonValue


@runtime_checkable
class WindowCache(Protocol):
    dual_cache: DualCacheHolder

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


@runtime_checkable
class LuaScript(Protocol):
    async def __call__(self, *, keys: Sequence[str], args: Sequence[int]) -> list[int | str]: ...


@runtime_checkable
class AtomicLimiter(Protocol):
    check_and_increment_by_n_script: LuaScript | None  # ast-grep-ignore: no-long-names

    async def atomic_check_and_increment_by_n(  # ast-grep-ignore: no-long-names
        self, descriptors: list[Descriptor], increments: list[IncrementAmounts]
    ) -> ResponseOut: ...


class NoRedisCache:
    redis_cache: JsonValue = None


class RecordingCache(WindowCache):
    def __init__(self, values: dict[str, int | str]) -> None:
        self.dual_cache: DualCacheHolder = NoRedisCache()
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


class RecordingScript(LuaScript):
    def __init__(self, reply: list[int | str]) -> None:
        self.reply = reply
        self.calls: list[CallJson] = []

    @override
    async def __call__(self, *, keys: Sequence[str], args: Sequence[int]) -> list[int | str]:
        self.calls.append(CallJson(keys=list(keys), args=list(args)))
        return self.reply


class FixedTime:
    def __init__(self, epoch: int) -> None:
        self.epoch = epoch

    def __call__(self) -> datetime:
        return datetime.fromtimestamp(self.epoch, tz=UTC)


def make_limiter(cache: WindowCache, time_provider: Callable[[], datetime]) -> AtomicLimiter:
    constructor = TypeAdapter(
        Callable[[WindowCache, Callable[[], datetime]], AtomicLimiter]
    ).validate_python(PROXY_HOOKS["parallel_request_limiter"])
    return constructor(cache, time_provider)


def status_value(status: StatusOut) -> StatusJson:
    return StatusJson(
        code=status["code"],
        current_limit=status["current_limit"],
        limit_remaining=status["limit_remaining"],
        rate_limit_type=status["rate_limit_type"],
        descriptor_key=status["descriptor_key"],
        descriptor_value=status["descriptor_value"],
    )


def window_value(window: ReservationWindow) -> WindowJson:
    counter_key, window_start, backend = window
    return WindowJson(counter_key=counter_key, window_start=window_start, backend=backend)


def response_value(response: ResponseOut) -> ResponseJson:
    reserved = response.get("reservation_windows")
    windows = [] if reserved is None else [window_value(window) for window in sorted(reserved)]
    return {
        "overall_code": response["overall_code"],
        "statuses": [status_value(status) for status in response["statuses"]],
        "reservation_windows": windows,
    }


def run(input_value: AtomicCheckInput) -> Answer:
    cache = RecordingCache(input_value["cache"])
    limiter = make_limiter(cache, FixedTime(input_value["now"]))
    script = None if input_value["reply"] is None else RecordingScript(input_value["reply"])
    limiter.check_and_increment_by_n_script = script
    response = asyncio.run(
        limiter.atomic_check_and_increment_by_n(
            [input_value["descriptor"]], [input_value["increment"]]
        )
    )
    return Answer(
        response=response_value(response),
        calls=[] if script is None else script.calls,
        reads=cache.reads,
        writes=cache.writes,
    )
