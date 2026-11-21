from collections.abc import Callable
from typing import Protocol, TypedDict

from litellm.proxy.hooks import PROXY_HOOKS
from litellm.proxy.hooks.parallel_request_limiter_v3 import CallTypeRateLimiter
from pydantic import JsonValue, TypeAdapter


class DualCacheHolder(Protocol):
    redis_cache: JsonValue


class InternalUsageCache(Protocol):
    dual_cache: DualCacheHolder


class CallTypeSelector(Protocol):
    def get_rate_limiter_for_call_type(self, call_type: str) -> CallTypeRateLimiter | None: ...


class NoRedisCache:
    redis_cache: JsonValue = None


class FakeCache:
    dual_cache: DualCacheHolder

    def __init__(self) -> None:
        self.dual_cache = NoRedisCache()


class SelectCallTypeInput(TypedDict):
    call_type: str


def run(input_value: SelectCallTypeInput) -> dict[str, JsonValue]:
    constructor = TypeAdapter(
        Callable[[InternalUsageCache], CallTypeSelector]
    ).validate_python(PROXY_HOOKS["parallel_request_limiter"])
    limiter = constructor(FakeCache())
    selected = limiter.get_rate_limiter_for_call_type(input_value["call_type"])
    return {"choice": "batch" if selected is not None else "main"}
