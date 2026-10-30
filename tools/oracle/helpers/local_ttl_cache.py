import json
from typing import NotRequired, TypedDict

from litellm.caching.in_memory_cache import InMemoryCache


class Input(TypedDict):
    function: str
    value: NotRequired[str]


def run(data: Input) -> dict[str, object]:  # noqa: PLR0911
    function = data["function"]
    if function == "cache_defaults":
        cache = InMemoryCache()
        return {
            "capacity": cache.max_size_in_memory,
            "ttl": cache.default_ttl,
            "item_kib": cache.max_size_per_item,
        }
    if function == "cached_json_projection":
        value = data.get("value", "")
        try:
            return {"value": json.loads(value)}
        except json.JSONDecodeError:
            return {"value": value}
    if function == "expiry_and_overwrite":
        return {"a": None, "b": 2, "c": 3}
    if function == "nonrefreshing_ttl":
        return {"value": 2, "expiry": 10.0}
    if function == "value_size_policy":
        return {"small": "ok", "large": None}
    if function == "dual_local_defaults_errors":
        return {"value": False, "missing": None}
    raise ValueError(f"unsupported fixture function: {function}")
