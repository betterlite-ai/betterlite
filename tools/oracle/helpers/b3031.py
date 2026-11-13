import asyncio
from collections.abc import Callable
from typing import Literal, Protocol, TypeAlias, TypedDict, runtime_checkable

from litellm.proxy.hooks import PROXY_HOOKS
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    RateLimitedModel,
    fail_closed_rate_limit_enforcement_enabled,
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

LimitType: TypeAlias = Literal["requests", "tokens", "max_parallel_requests"]
CounterValue: TypeAlias = int | float | str | bytes | None


class FlagConfig(TypedDict):
    general_settings: dict[str, JsonValue]


class CounterKeysInput(TypedDict):
    function: Literal["counter_keys"]
    key: str
    value: str
    rate_limit_type: LimitType


class SiblingCounterKeysInput(TypedDict):
    function: Literal["sibling_counter_keys"]
    window_key: str


class ModelLimitInput(TypedDict):
    function: Literal["model_limit"]
    requested: str
    group: str
    limits: dict[str, int] | None


class FailClosedFlagInput(TypedDict):
    function: Literal["fail_closed_flag"]
    config: FlagConfig


Input: TypeAlias = (
    CounterKeysInput | SiblingCounterKeysInput | ModelLimitInput | FailClosedFlagInput
)


@runtime_checkable
class KeyBuilder(Protocol):
    def create_rate_limit_keys(
        self, key: str, value: str, rate_limit_type: LimitType
    ) -> str: ...


class DualCacheHolder(Protocol):
    redis_cache: JsonValue


@runtime_checkable
class WindowCache(Protocol):
    dual_cache: DualCacheHolder

    async def async_get_cache(
        self, key: str, *, litellm_parent_otel_span: JsonValue, local_only: bool
    ) -> JsonValue: ...

    async def async_set_cache(
        self,
        key: str,
        value: JsonValue,
        ttl: int,
        *,
        litellm_parent_otel_span: JsonValue,
        local_only: bool,
    ) -> None: ...


@runtime_checkable
class SlidingWindow(Protocol):
    async def in_memory_cache_sliding_window(
        self, keys: list[str], now_int: int, window_size: int
    ) -> list[CounterValue]: ...


class NoRedisCache:
    redis_cache: JsonValue = None


class RecordingCache(WindowCache):
    def __init__(self) -> None:
        self.dual_cache: DualCacheHolder = NoRedisCache()
        self.reads: list[tuple[str, JsonValue, bool]] = []
        self.writes: list[tuple[str, JsonValue, int, JsonValue, bool]] = []

    @override
    async def async_get_cache(
        self, key: str, *, litellm_parent_otel_span: JsonValue, local_only: bool
    ) -> JsonValue:
        self.reads.append((key, litellm_parent_otel_span, local_only))
        return None

    @override
    async def async_set_cache(
        self,
        key: str,
        value: JsonValue,
        ttl: int,
        *,
        litellm_parent_otel_span: JsonValue,
        local_only: bool,
    ) -> None:
        self.writes.append((key, value, ttl, litellm_parent_otel_span, local_only))


def sibling_keys(window_key: str) -> dict[str, JsonValue]:
    cache = RecordingCache()
    constructor = TypeAdapter(
        Callable[[WindowCache], SlidingWindow]
    ).validate_python(PROXY_HOOKS["parallel_request_limiter"])
    limiter = constructor(cache)
    asyncio.run(
        limiter.in_memory_cache_sliding_window(
            [window_key, "counter-key"], 1_700_000_000, 60
        )
    )
    return {
        "requests_key": cache.writes[0][0],
        "tokens_key": cache.writes[1][0],
    }


def run(input_value: Input) -> dict[str, JsonValue]:
    if input_value["function"] == "counter_keys":
        hook_class = TypeAdapter(type[KeyBuilder]).validate_python(
            PROXY_HOOKS["parallel_request_limiter"]
        )
        limiter = object.__new__(hook_class)
        return {
            "counter_key": limiter.create_rate_limit_keys(
                input_value["key"], input_value["value"], input_value["rate_limit_type"]
            )
        }
    if input_value["function"] == "sibling_counter_keys":
        return sibling_keys(input_value["window_key"])
    if input_value["function"] == "model_limit":
        model = RateLimitedModel(
            requested=input_value["requested"], group=input_value["group"]
        )
        return {"limit": model.limit_in(input_value["limits"])}
    return {
        "enabled": fail_closed_rate_limit_enforcement_enabled(
            input_value["config"]["general_settings"]
        )
    }
