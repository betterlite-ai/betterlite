from typing import NotRequired, Protocol, TypedDict, cast
from unittest.mock import patch

from litellm.caching.dual_cache import DualCache
from litellm.caching.in_memory_cache import InMemoryCache


class Cache(Protocol):
    max_size_in_memory: int
    default_ttl: int
    max_size_per_item: int

    def set_cache(self, key: str, value: object, **kwargs: object) -> None: ...
    def get_cache(self, key: str) -> object: ...
    def delete_cache(self, key: str) -> None: ...


class Input(TypedDict):
    function: str
    value: NotRequired[str]


def run(data: Input) -> dict[str, object]:
    function = data["function"]
    now = [0.0]
    cache = cast("Cache", InMemoryCache(max_size_in_memory=2, clock=lambda: now[0]))
    if function == "cache_defaults":
        defaults = cast("Cache", InMemoryCache(clock=lambda: now[0]))
        zero = cast(
            "Cache", InMemoryCache(default_ttl=0, max_size_per_item=0, clock=lambda: now[0])
        )
        return {
            "capacity": defaults.max_size_in_memory,
            "ttl": float(defaults.default_ttl),
            "item_kib": defaults.max_size_per_item,
            "zero_ttl": float(zero.default_ttl),
            "zero_item_kib": zero.max_size_per_item,
        }
    if function == "cached_json_projection":
        cache.set_cache("value", data.get("value", ""))
        return {"value": cache.get_cache("value")}
    if function == "expiry_and_overwrite":
        now[0] = 10.0
        cache.set_cache("b", 2, ttl=5)
        cache.set_cache("a", 1, ttl=5)
        cache.set_cache("c", 3, ttl=5)
        return {key: cache.get_cache(key) for key in ("a", "b", "c")}
    if function == "nonrefreshing_ttl":
        cache.set_cache("key", 1, ttl=10)
        cache.set_cache("key", 2, ttl=100)
        now[0] = 10.0
        boundary = cache.get_cache("key")
        now[0] = 11.0
        expired = cache.get_cache("key")
        cache.set_cache("deleted", 3)
        cache.delete_cache("deleted")
        return {
            "boundary": boundary,
            "expired": expired,
            "deleted": cache.get_cache("deleted"),
        }
    if function == "value_size_policy":
        small_cache = cast("Cache", InMemoryCache(max_size_per_item=1, clock=lambda: now[0]))
        small_cache.set_cache("small", "ok")
        small_cache.set_cache("large", "x" * 2000)
        return {
            "small": small_cache.get_cache("small"),
            "large": small_cache.get_cache("large"),
        }
    if function == "dual_local_defaults_errors":
        return dual_defaults(cache, now)
    raise ValueError(f"unsupported fixture function: {function}")


def dual_defaults(cache: Cache, now: list[float]) -> dict[str, object]:
    dual = cast(
        "Cache", DualCache(in_memory_cache=cast("InMemoryCache", cache), default_in_memory_ttl=4)
    )
    dual.set_cache("false", value=False)
    dual.set_cache("zero", 0, ttl=None)
    now[0] = 5.0
    expired = dual.get_cache("false")
    zero = dual.get_cache("zero")
    with (
        patch.object(cache, "set_cache", side_effect=RuntimeError("fixture-secret")),
        patch("litellm.caching.dual_cache.print_verbose") as verbose,
    ):
        dual.set_cache("failure", "fixture-secret")
        set_event = verbose.called
    with (
        patch.object(cache, "get_cache", side_effect=RuntimeError("fixture-secret")),
        patch("litellm.caching.dual_cache.log_redis_failure") as log,
    ):
        failed = dual.get_cache("failure")
        get_event = log.called
    return {
        "expired": expired,
        "zero": zero,
        "failure": failed,
        "set_event": set_event,
        "get_event": get_event,
    }
