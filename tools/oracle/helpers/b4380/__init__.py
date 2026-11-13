from operator import attrgetter
from typing import Protocol, TypeAlias, cast

from litellm.caching.redis_semantic_cache import RedisSemanticCache

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class _PromptMethod(Protocol):
    def __call__(self, **kwargs: Json) -> str | None: ...


class _CacheDecoder(Protocol):
    def __call__(self, payload: str) -> object: ...


def _extract_semantic_prompt(input_value: dict[str, Json]) -> Json:
    kwargs: dict[str, Json] = {
        key: value
        for key, value in input_value.items()
        if key in {"messages", "input"}
    }
    method = cast(_PromptMethod, attrgetter("_get_prompt_from_kwargs")(RedisSemanticCache))
    result = method(**kwargs)
    return cast(Json, result)


def _decode_cached_response(input_value: dict[str, Json]) -> Json:
    payload = cast(str, input_value["payload"])
    cache = object.__new__(RedisSemanticCache)
    method = cast(_CacheDecoder, attrgetter("_get_cache_logic")(cache))
    result = method(payload)
    return cast(Json, result)


def run(input_value: dict[str, Json]) -> Json:
    operation = cast(str, input_value["operation"])
    if operation == "extract_semantic_prompt":
        return _extract_semantic_prompt(input_value)
    if operation == "decode_cached_response":
        return _decode_cached_response(input_value)
    raise ValueError(f"unsupported operation {operation}")
