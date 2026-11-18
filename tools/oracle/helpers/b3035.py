import asyncio
from collections.abc import Sequence
from datetime import UTC, datetime
from typing import Literal, NamedTuple, Protocol, TypedDict, runtime_checkable

from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.hooks import PROXY_HOOKS
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    ParallelSlotAcquisition,
    RateLimitDescriptor,
    RateLimitResponse,
    RateLimitStatus,
    get_or_create_request_stash,
)
from pydantic import ConfigDict, JsonValue, TypeAdapter


class ReplyInput(TypedDict, total=False):
    values: list[int | str]
    failure: str


class GaugeCheckInput(TypedDict):
    function: Literal["gauge_check"]
    descriptors: list[RateLimitDescriptor]
    slot_id: str
    read_only: bool
    fail_closed: bool
    now: int
    cache: dict[str, JsonValue]
    replies: list[ReplyInput] | None


class ReleaseInput(TypedDict):
    function: Literal["release"]
    counter_keys: list[str]
    slot_id: str
    now: int
    cache: dict[str, JsonValue]
    replies: list[ReplyInput] | None


class PlannedScript:
    def __init__(self, plan: list[ReplyInput]) -> None:
        self.plan = plan
        self.calls: list[dict[str, JsonValue]] = []

    async def __call__(self, *, keys: Sequence[str], args: Sequence[JsonValue]) -> list[int | str]:
        index = len(self.calls)
        self.calls.append({"keys": list(keys), "args": [str(arg) for arg in args]})
        if index >= len(self.plan):
            raise RuntimeError("the script plan ran out of replies")
        entry = self.plan[index]
        failure = entry.get("failure")
        if failure is not None:
            raise ConnectionError(failure)
        values = entry.get("values")
        if values is None:
            raise ValueError("a reply with no failure carries values")
        return values


class FakeDualCache(NamedTuple):
    redis_cache: JsonValue


class RecordingCache:
    def __init__(self, values: dict[str, JsonValue]) -> None:
        self.dual_cache = FakeDualCache(redis_cache=None)
        self.values = values
        self.reads: list[dict[str, JsonValue]] = []
        self.batch_reads: list[dict[str, JsonValue]] = []
        self.writes: list[dict[str, JsonValue]] = []

    async def async_get_cache(
        self, key: str, *, litellm_parent_otel_span: JsonValue, local_only: bool
    ) -> JsonValue | None:
        self.reads.append({"key": key, "local_only": local_only})
        return self.values.get(key)

    async def async_set_cache(
        self,
        key: str,
        value: JsonValue,
        ttl: int,
        *,
        litellm_parent_otel_span: JsonValue,
        local_only: bool,
    ) -> None:
        self.writes.append({"key": key, "value": value, "ttl": ttl, "local_only": local_only})
        self.values[key] = value

    async def async_batch_get_cache(
        self, *, keys: Sequence[str], parent_otel_span: JsonValue = None, local_only: bool = False
    ) -> list[JsonValue | None] | None:
        self.batch_reads.append({"keys": list(keys), "local_only": local_only})
        return [self.values.get(key) for key in keys]


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


class GaugeLimiter(Protocol):
    parallel_acquire_script: PlannedScript | None
    parallel_release_script: PlannedScript | None
    parallel_count_script: PlannedScript | None

    async def should_rate_limit(
        self,
        descriptors: Sequence[RateLimitDescriptor],
        *,
        read_only: bool,
        parallel_slot_id: str,
    ) -> RateLimitResponse: ...

    async def async_release_max_parallel_requests_on_disconnect(  # ast-grep-ignore: no-long-names
        self, user_api_key_dict: UserAPIKeyAuth
    ) -> None: ...


@runtime_checkable
class LimiterConstructor(Protocol):
    def __call__(
        self, cache: RecordingCache, clock: FixedTime, *, fail_closed_resolver: FixedFailClosed
    ) -> GaugeLimiter: ...


def make_limiter(
    cache: RecordingCache, clock: FixedTime, fail_closed: FixedFailClosed
) -> GaugeLimiter:
    adapter = TypeAdapter[LimiterConstructor](
        LimiterConstructor, config=ConfigDict(arbitrary_types_allowed=True)
    )
    constructor = adapter.validate_python(PROXY_HOOKS["parallel_request_limiter"])
    return constructor(cache, clock, fail_closed_resolver=fail_closed)


def status_value(status: RateLimitStatus) -> dict[str, JsonValue]:
    return {
        "code": status["code"],
        "current_limit": status["current_limit"],
        "limit_remaining": status["limit_remaining"],
        "rate_limit_type": status["rate_limit_type"],
        "descriptor_key": status["descriptor_key"],
    }


def response_value(response: RateLimitResponse) -> dict[str, JsonValue]:
    return {
        "overall_code": response["overall_code"],
        "statuses": [status_value(status) for status in response["statuses"]],
        "reservation_windows": [],
    }


async def gauge_check(input_value: GaugeCheckInput) -> dict[str, JsonValue]:
    cache = RecordingCache(input_value["cache"])
    limiter = make_limiter(
        cache,
        FixedTime(input_value["now"]),
        FixedFailClosed(enabled=input_value["fail_closed"]),
    )
    replies = input_value["replies"]
    script = PlannedScript(replies) if replies is not None else None
    limiter.parallel_acquire_script = script
    limiter.parallel_count_script = script
    response = await limiter.should_rate_limit(
        input_value["descriptors"],
        read_only=input_value["read_only"],
        parallel_slot_id=input_value["slot_id"],
    )
    return {
        "response": response_value(response),
        "calls": script.calls if script is not None else [],
        "reads": cache.reads,
        "batch_reads": cache.batch_reads,
        "writes": cache.writes,
    }


async def release_slot(input_value: ReleaseInput) -> dict[str, JsonValue]:
    cache = RecordingCache(input_value["cache"])
    limiter = make_limiter(
        cache, FixedTime(input_value["now"]), FixedFailClosed(enabled=False)
    )
    replies = input_value["replies"]
    script = PlannedScript(replies) if replies is not None else None
    limiter.parallel_release_script = script
    stash = get_or_create_request_stash()
    stash.parallel_slot = ParallelSlotAcquisition(
        slot_id=input_value["slot_id"], counter_keys=input_value["counter_keys"]
    )
    await limiter.async_release_max_parallel_requests_on_disconnect(UserAPIKeyAuth())
    return {
        "response": None,
        "calls": script.calls if script is not None else [],
        "reads": cache.reads,
        "batch_reads": cache.batch_reads,
        "writes": cache.writes,
    }


def run(input_value: GaugeCheckInput | ReleaseInput) -> dict[str, JsonValue]:
    with asyncio.Runner() as runner:
        if input_value["function"] == "release":
            return runner.run(release_slot(input_value))
        return runner.run(gauge_check(input_value))
