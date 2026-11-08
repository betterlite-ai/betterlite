import ast
import asyncio
import json
from collections.abc import Callable
from operator import attrgetter
from typing import Protocol, TypeAlias, cast

from litellm.caching.redis_semantic_cache import RedisSemanticCache

_URL = "redis://:pw@localhost:6379"

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
    def __init__(self, mismatches: set[int]) -> None:
        self.mismatches = mismatches
        self.attempts: list[Json] = []

    def __call__(self, **kwargs: object) -> dict[str, Json]:
        name = cast(str, kwargs["name"])
        overwrite = cast(Json, kwargs["overwrite"])
        self.attempts.append({"name": name, "overwrite": overwrite})
        if len(self.attempts) - 1 in self.mismatches:
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
        filter_expression = cast(dict[str, Json], kwargs["filter_expression"])
        self.checks.append(
            {
                "cache_key": filter_expression["cache_key"],
                "vector": cast(Json, kwargs["vector"]),
            }
        )
        return self.hits

    async def astore(self, prompt: str, response: str, **kwargs: object) -> str:
        ttl = kwargs.get("ttl")
        self.stores.append(
            {
                "prompt": prompt,
                "response": _normalized_response(response),
                "vector": cast(Json, kwargs["vector"]),
                "filters": cast(Json, kwargs["filters"]),
                "ttl": cast(Json, ttl) if ttl else None,
            }
        )
        return "stored"


class _ScriptedCache(RedisSemanticCache):
    def __init__(self, embedding: list[float], llm: _FakeLLMCache) -> None:
        super().__init__(similarity_threshold=0.25, redis_url=_URL)
        self.llmcache = llm
        self.embedding = embedding

    def _get_cache_key_filter_expression(self, key: str) -> object:
        return {"cache_key": key}

    async def _get_async_embedding(
        self, prompt: str, metadata: dict[str, object] | None = None
    ) -> list[float]:
        del prompt, metadata
        return self.embedding


def _normalized_response(stored: str) -> str:
    parsed: object
    try:
        parsed = cast(object, json.loads(stored))
    except json.JSONDecodeError:
        parsed = cast(object, ast.literal_eval(stored))
    return json.dumps(parsed, separators=(",", ":"))


def _object(value: Json) -> dict[str, Json]:
    return cast(dict[str, Json], value)


def _floats(value: Json) -> list[float]:
    return [float(item) for item in cast(list[float], value)]


def _attribute(cache: object, name: str) -> Json:
    return cast(Json, attrgetter(name)(cache))


def _constructor(input_value: dict[str, Json]) -> Json:
    names = (
        "host",
        "port",
        "password",
        "redis_url",
        "index_name",
        "embedding_max_input_tokens",
    )
    kwargs: dict[str, object] = {name: input_value.get(name) for name in names}
    kwargs["similarity_threshold"] = input_value.get("threshold")
    kwargs["embedding_timeout"] = input_value.get("embedding_timeout")
    model = input_value.get("embedding_model")
    if model is not None:
        kwargs["embedding_model"] = model
    cache = cast(Callable[..., object], RedisSemanticCache)(**kwargs)
    fields = (
        ("index_name", "_index_name"),
        ("distance_threshold", "distance_threshold"),
        ("redis_url", "_redis_url"),
        ("embedding_model", "embedding_model"),
        ("embedding_max_input_tokens", "embedding_max_input_tokens"),
        ("embedding_timeout", "embedding_timeout"),
    )
    return {name: _attribute(cache, source) for name, source in fields}


def _key_scope(input_value: dict[str, Json]) -> Json:
    cache = cast(object, object.__new__(RedisSemanticCache))
    method = cast(_KeyMatch, attrgetter("_cache_hit_matches_key")(cache))
    hit: dict[str, object] = {"litellm_cache_key": input_value.get("litellm_cache_key")}
    return {"matches": method(hit, cast(str, input_value.get("key")))}


def _index_fallback(input_value: dict[str, Json]) -> Json:
    attempts = cast(list[int], input_value.get("mismatch_attempts", []))
    fake = _ScriptedSemanticCache(set(attempts))
    cache = cast(object, RedisSemanticCache(similarity_threshold=0.25, redis_url=_URL))
    method = cast(_InitMethod, attrgetter("_init_semantic_cache")(cache))
    result = method(
        fake,
        cast(str, input_value.get("index_name")),
        "redis://:pw@localhost:6379",
        None,
    )
    return {"chosen": _object(cast(Json, result))["index_name"], "attempts": fake.attempts}


def _cache_kwargs(
    input_value: dict[str, Json], metadata: dict[str, Json]
) -> dict[str, Json]:
    kwargs: dict[str, Json] = {}
    for key in ("messages", "input"):
        if key in input_value:
            kwargs[key] = input_value[key]
    if "metadata" in input_value:
        kwargs["metadata"] = metadata
    if input_value.get("ttl") is not None:
        kwargs["ttl"] = input_value["ttl"]
    return kwargs


def _lookup(input_value: dict[str, Json]) -> Json:
    hits = [_hit(item) for item in cast(list[Json], input_value.get("hits", []))]
    llm = _FakeLLMCache(hits)
    cache = _ScriptedCache(_floats(input_value["vector"]), llm)
    metadata = _object(input_value["metadata"]) if "metadata" in input_value else {}
    kwargs = _cache_kwargs(input_value, metadata)
    ops = cast(_CacheOps, cache)
    value = cast(
        Json,
        asyncio.run(ops.async_get_cache(cast(str, input_value.get("key")), **kwargs)),
    )
    check = llm.checks[0] if llm.checks else None
    search: Json = (
        None
        if check is None
        else {
            "cache_key": check["cache_key"],
            "vector": check["vector"],
            "distance_threshold": cache.distance_threshold,
        }
    )
    return {
        "value": value,
        "similarity": metadata.get("semantic-similarity") if "metadata" in kwargs else None,
        "search": search,
    }


def _hit(value: Json) -> dict[str, Json]:
    hit = _object(value)
    return {
        key: hit[key]
        for key in ("prompt", "response", "litellm_cache_key", "vector_distance")
        if key in hit
    }


def _store(input_value: dict[str, Json]) -> Json:
    llm = _FakeLLMCache([])
    cache = _ScriptedCache(_floats(input_value["vector"]), llm)
    metadata = _object(input_value["metadata"]) if "metadata" in input_value else {}
    kwargs = _cache_kwargs(input_value, metadata)
    ops = cast(_CacheOps, cache)
    asyncio.run(
        ops.async_set_cache(
            cast(str, input_value.get("key")), input_value["value"], **kwargs
        )
    )
    return {"store": llm.stores[0] if llm.stores else None}


def run(input_value: dict[str, Json]) -> Json:
    operation = input_value.get("operation")
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
