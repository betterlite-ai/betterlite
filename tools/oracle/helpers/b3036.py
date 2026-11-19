from collections.abc import Sequence
from typing import Literal, NamedTuple, Protocol, TypedDict, TypeAlias, runtime_checkable

from litellm.proxy.hooks import PROXY_HOOKS
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    CacheCounterValues,
    ParallelRequestGauge,
    RateLimitDescriptor,
    RateLimitResponse,
    WindowKeyMetadata,
)
from pydantic import ConfigDict, JsonValue, TypeAdapter


class CollectInput(TypedDict):
    function: Literal["collect"]
    descriptors: list[RateLimitDescriptor]
    skip_tpm_check: bool
    window_size: int


class OverLimitInput(TypedDict):
    function: Literal["check_over_limit"]
    keys_to_fetch: list[str]
    cache_values: list[int | float | str | None]
    key_metadata: dict[str, WindowKeyMetadata]
    read_only: bool


Input: TypeAlias = CollectInput | OverLimitInput


class FakeDualCache(NamedTuple):
    redis_cache: None


class StandInCache:
    def __init__(self) -> None:
        self.dual_cache = FakeDualCache(redis_cache=None)


class WindowedLimiter(Protocol):
    window_size: int

    def _collect_windowed_keys_and_gauges(
        self,
        descriptors: Sequence[RateLimitDescriptor],
        skip_tpm_check: bool,
    ) -> tuple[list[str], dict[str, WindowKeyMetadata], list[ParallelRequestGauge]]: ...

    def is_cache_list_over_limit(
        self,
        keys_to_fetch: list[str],
        cache_values: CacheCounterValues,
        key_metadata: dict[str, WindowKeyMetadata],
        read_only: bool = False,
    ) -> RateLimitResponse: ...


@runtime_checkable
class LimiterConstructor(Protocol):
    def __call__(self, cache: StandInCache) -> WindowedLimiter: ...


def make_limiter() -> WindowedLimiter:
    adapter = TypeAdapter[LimiterConstructor](
        LimiterConstructor, config=ConfigDict(arbitrary_types_allowed=True)
    )
    constructor = adapter.validate_python(PROXY_HOOKS["parallel_request_limiter"])
    return constructor(StandInCache())


def metadata_value(metadata: dict[str, WindowKeyMetadata]) -> dict[str, JsonValue]:
    result: dict[str, JsonValue] = {}
    for window_key, entry in metadata.items():
        result[window_key] = {
            "requests_limit": entry["requests_limit"],
            "tokens_limit": entry["tokens_limit"],
            "window_size": entry["window_size"],
            "descriptor_key": entry["descriptor_key"],
            "descriptor_value": entry["descriptor_value"],
        }
    return result


def gauges_value(gauges: Sequence[ParallelRequestGauge]) -> list[JsonValue]:
    result: list[JsonValue] = []
    for gauge in gauges:
        result.append(
            {
                "counter_key": gauge["counter_key"],
                "limit": gauge["limit"],
                "descriptor_key": gauge["descriptor_key"],
            }
        )
    return result


def response_value(response: RateLimitResponse) -> dict[str, JsonValue]:
    statuses: list[JsonValue] = []
    for status in response["statuses"]:
        statuses.append(
            {
                "code": status["code"],
                "current_limit": status["current_limit"],
                "limit_remaining": status["limit_remaining"],
                "rate_limit_type": status["rate_limit_type"],
                "descriptor_key": status["descriptor_key"],
                "descriptor_value": status.get("descriptor_value"),
            }
        )
    return {
        "overall_code": response["overall_code"],
        "statuses": statuses,
        "reservation_windows": None,
    }


def collect(input_value: CollectInput) -> dict[str, JsonValue]:
    limiter = make_limiter()
    limiter.window_size = input_value["window_size"]
    keys, metadata, gauges = limiter._collect_windowed_keys_and_gauges(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/parallel_request_limiter_v3.py#L1657-L1714
        descriptors=input_value["descriptors"],
        skip_tpm_check=input_value["skip_tpm_check"],
    )
    keys_value: list[JsonValue] = []
    for key in keys:
        keys_value.append(key)
    return {
        "keys_to_fetch": keys_value,
        "key_metadata": metadata_value(metadata),
        "gauges": gauges_value(gauges),
    }


def check_over_limit(input_value: OverLimitInput) -> dict[str, JsonValue]:
    response = make_limiter().is_cache_list_over_limit(
        input_value["keys_to_fetch"],
        input_value["cache_values"],
        input_value["key_metadata"],
        read_only=input_value["read_only"],
    )
    return response_value(response)


def run(input_value: Input) -> dict[str, JsonValue]:
    if input_value["function"] == "collect":
        return collect(input_value)
    return check_over_limit(input_value)
