import ast
import asyncio
import json
from collections.abc import Callable
from operator import attrgetter
from typing import Protocol, TypeAlias, cast

from litellm.caching.redis_semantic_cache import RedisSemanticCache

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class _InitMethod(Protocol):
    def __call__(
        self,
        semantic_cache_cls: Callable[..., object],
        index_name: str,
        redis_url: str,
        cache_vectorizer: object,
    ) -> object: ...


class _KeyMatch(Protocol):
    def __call__(self, cache_hit: dict[str, object], key: str) -> bool: ...


class _CacheOps(Protocol):
    async def async_get_cache(self, key: str, **kwargs: Json) -> object: ...

    async def async_set_cache(self, key: str, value: object, **kwargs: Json) -> None: ...


class _ScriptedSemanticCache:
    def __init__(self, mismatch_attempts: set[int]) -> None:
        self.mismatch_attempts = mismatch_attempts
        self.attempts: list[Json] = []

    def __call__(self, **kwargs: object) -> dict[str, Json]:
        name = _text(cast(Json, kwargs["name"]), "name")
        overwrite = cast(Json, kwargs["overwrite"])
        self.attempts.append({"name": name, "overwrite": overwrite})
        if len(self.attempts) - 1 in self.mismatch_attempts:
            raise ValueError(
                f"Existing index {name} schema does not match the user provided schema"
            )
        return {"index_name": name}


class _FakeLLMCache:
    def __init__(self, hits: list[dict[str, Json]]) -> None:
        self.hits = hits
        self.checks: list[dict[str, Json]] = []
        self.stores: list[dict[str, Json]] = []

    async def acheck(self, **kwargs: object) -> list[dict[str, Json]]:
        self.checks.append(_check_record(kwargs))
        return self.hits

    async def astore(self, prompt: str, response: str, **kwargs: object) -> str:
        self.stores.append(_store_record(prompt, response, kwargs))
        return "stored"


class _ScriptedCache(RedisSemanticCache):
    def __init__(self, embedding: list[float], llm: _FakeLLMCache) -> None:
        super().__init__(similarity_threshold=0.25, redis_url="redis://:pw@localhost:6379")
        self.llmcache = llm
        self.embedding = embedding
        self.prompts: list[str] = []

    def _get_cache_key_filter_expression(self, key: str) -> object:
        return {"cache_key": key}

    async def _get_async_embedding(
        self, prompt: str, metadata: dict[str, object] | None = None
    ) -> list[float]:
        self.prompts.append(prompt)
        del metadata
        return self.embedding


def _check_record(kwargs: dict[str, object]) -> dict[str, Json]:
    filter_expression = cast(dict[str, Json], kwargs["filter_expression"])
    return {
        "cache_key": filter_expression["cache_key"],
        "vector": cast(Json, kwargs["vector"]),
    }


def _store_record(
    prompt: str, response: str, kwargs: dict[str, object]
) -> dict[str, Json]:
    ttl = kwargs.get("ttl")
    return {
        "prompt": prompt,
        "response": _normalized_response(response),
        "vector": cast(Json, kwargs["vector"]),
        "filters": cast(Json, kwargs["filters"]),
        "ttl": cast(Json, ttl) if ttl else None,
    }


def _normalized_response(stored: str) -> str:
    parsed: object
    try:
        parsed = cast(object, json.loads(stored))
    except json.JSONDecodeError:
        parsed = cast(object, ast.literal_eval(stored))
    return json.dumps(parsed, separators=(",", ":"))


def _text(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _optional_text(value: Json, name: str) -> str | None:
    if value is None:
        return None
    return _text(value, name)


def _number(value: Json, name: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be a number")
    return float(value)


def _optional_number(value: Json, name: str) -> float | None:
    if value is None:
        return None
    return _number(value, name)


def _integer(value: Json, name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{name} must be an integer")
    return value


def _optional_integer(value: Json, name: str) -> int | None:
    if value is None:
        return None
    return _integer(value, name)


def _list(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def _object(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def _floats(value: Json, name: str) -> list[float]:
    return [_number(item, name) for item in _list(value, name)]


def _integers(value: Json, name: str) -> list[int]:
    return [_integer(item, name) for item in _list(value, name)]


def _attribute(cache: object, name: str) -> Json:
    return cast(Json, attrgetter(name)(cache))


def _constructor(input_value: dict[str, Json]) -> Json:
    kwargs: dict[str, object] = {
        "host": _optional_text(input_value.get("host"), "host"),
        "port": _optional_text(input_value.get("port"), "port"),
        "password": _optional_text(input_value.get("password"), "password"),
        "redis_url": _optional_text(input_value.get("redis_url"), "redis_url"),
        "similarity_threshold": _optional_number(input_value.get("threshold"), "threshold"),
        "index_name": _optional_text(input_value.get("index_name"), "index_name"),
        "embedding_max_input_tokens": _optional_integer(
            input_value.get("embedding_max_input_tokens"), "embedding_max_input_tokens"
        ),
        "embedding_timeout": _optional_number(
            input_value.get("embedding_timeout"), "embedding_timeout"
        ),
    }
    model = _optional_text(input_value.get("embedding_model"), "embedding_model")
    if model is not None:
        kwargs["embedding_model"] = model
    constructor = cast(Callable[..., object], RedisSemanticCache)
    cache = constructor(**kwargs)
    return {
        "index_name": _attribute(cache, "_index_name"),
        "distance_threshold": _attribute(cache, "distance_threshold"),
        "redis_url": _attribute(cache, "_redis_url"),
        "embedding_model": _attribute(cache, "embedding_model"),
        "embedding_max_input_tokens": _attribute(cache, "embedding_max_input_tokens"),
        "embedding_timeout": _attribute(cache, "embedding_timeout"),
    }


def _key_scope(input_value: dict[str, Json]) -> Json:
    cache = object.__new__(RedisSemanticCache)
    method = cast(_KeyMatch, attrgetter("_cache_hit_matches_key")(cache))
    hit: dict[str, object] = {"litellm_cache_key": input_value.get("litellm_cache_key")}
    return {"matches": method(hit, _text(input_value.get("key"), "key"))}


def _index_fallback(input_value: dict[str, Json]) -> Json:
    mismatch = set(
        _integers(input_value.get("mismatch_attempts", []), "mismatch_attempts")
    )
    fake = _ScriptedSemanticCache(mismatch)
    cache = RedisSemanticCache(
        similarity_threshold=0.25, redis_url="redis://:pw@localhost:6379"
    )
    method = cast(_InitMethod, attrgetter("_init_semantic_cache")(cache))
    result = method(
        fake,
        _text(input_value.get("index_name"), "index_name"),
        "redis://:pw@localhost:6379",
        None,
    )
    chosen = _object(cast(Json, result), "result")["index_name"]
    return {"chosen": chosen, "attempts": fake.attempts}


def _cache_kwargs(input_value: dict[str, Json], metadata: dict[str, Json]) -> dict[str, Json]:
    kwargs: dict[str, Json] = {}
    if "messages" in input_value:
        kwargs["messages"] = input_value["messages"]
    if "input" in input_value:
        kwargs["input"] = input_value["input"]
    if "metadata" in input_value:
        kwargs["metadata"] = metadata
    if input_value.get("ttl") is not None:
        kwargs["ttl"] = input_value["ttl"]
    return kwargs


def _scripted_cache(input_value: dict[str, Json], llm: _FakeLLMCache) -> _ScriptedCache:
    return _ScriptedCache(_floats(input_value["vector"], "vector"), llm)


def _lookup(input_value: dict[str, Json]) -> Json:
    hits = [_hit(hit) for hit in _list(input_value.get("hits", []), "hits")]
    llm = _FakeLLMCache(hits)
    cache = _scripted_cache(input_value, llm)
    metadata = (
        _object(input_value["metadata"], "metadata") if "metadata" in input_value else {}
    )
    kwargs = _cache_kwargs(input_value, metadata)
    key = _text(input_value.get("key"), "key")
    ops = cast(_CacheOps, cache)
    value = cast(Json, asyncio.run(ops.async_get_cache(key, **kwargs)))
    check = llm.checks[0] if llm.checks else None
    return {
        "value": value,
        "similarity": metadata.get("semantic-similarity") if "metadata" in kwargs else None,
        "search": (
            {
                "cache_key": check["cache_key"],
                "vector": check["vector"],
                "distance_threshold": cast(Json, cache.distance_threshold),
            }
            if check is not None
            else None
        ),
    }


def _hit(value: Json) -> dict[str, Json]:
    hit = _object(value, "hit")
    return {
        key: hit[key]
        for key in ("prompt", "response", "litellm_cache_key", "vector_distance")
        if key in hit
    }


def _store(input_value: dict[str, Json]) -> Json:
    llm = _FakeLLMCache([])
    cache = _scripted_cache(input_value, llm)
    metadata = (
        _object(input_value["metadata"], "metadata") if "metadata" in input_value else {}
    )
    kwargs = _cache_kwargs(input_value, metadata)
    key = _text(input_value.get("key"), "key")
    ops = cast(_CacheOps, cache)
    asyncio.run(ops.async_set_cache(key, input_value["value"], **kwargs))
    return {"store": llm.stores[0] if llm.stores else None}


def run(input_value: dict[str, Json]) -> Json:
    operation = _text(input_value.get("operation"), "operation")
    if operation == "constructor":
        return _constructor(input_value)
    if operation == "key_scope":
        return _key_scope(input_value)
    if operation == "index_fallback":
        return _index_fallback(input_value)
    if operation == "lookup":
        return _lookup(input_value)
    if operation == "store":
        return _store(input_value)
    raise ValueError(f"unsupported operation {operation}")
