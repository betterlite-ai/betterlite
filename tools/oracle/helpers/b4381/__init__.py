import asyncio
from typing import Protocol, TypeAlias, cast
from unittest.mock import patch

from litellm.caching._embedding_router import resolve_embedding_timeout
from litellm.caching.qdrant_semantic_cache import QdrantSemanticCache
from litellm.caching.redis_semantic_cache import RedisSemanticCache

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class _CallError(Exception):
    status_code = 500


class _CacheEmbeddings(Protocol):
    def _get_embedding(
        self, prompt: str, metadata: dict[str, object] | None = None
    ) -> object: ...

    async def _get_async_embedding(
        self, prompt: str, metadata: dict[str, object] | None = None
    ) -> object: ...


class _Recorder:
    def __init__(
        self, vectors: list[list[float]], model: str, *, failing: bool
    ) -> None:
        self.vectors = vectors
        self.model = model
        self.failing = failing
        self.calls: list[dict[str, Json]] = []

    def call(self, *, routed: bool, kwargs: dict[str, Json]) -> dict[str, Json]:
        self.calls.append(
            {
                "routed": routed,
                "model": kwargs["model"],
                "input": kwargs["input"],
                "cache": kwargs["cache"],
                "metadata": kwargs.get("metadata"),
                "timeout": kwargs["timeout"],
                "num_retries": kwargs["num_retries"],
            }
        )
        if self.failing:
            raise _CallError
        return cast(
            dict[str, Json],
            {
                "object": "list",
                "data": [
                    {"object": "embedding", "embedding": vector, "index": index}
                    for index, vector in enumerate(self.vectors)
                ],
                "model": self.model,
                "usage": {"prompt_tokens": 0, "total_tokens": 0},
            },
        )

    def direct_embedding(self, **kwargs: Json) -> dict[str, Json]:
        return self.call(routed=False, kwargs=kwargs)

    async def direct_aembedding(self, **kwargs: Json) -> dict[str, Json]:
        return self.call(routed=False, kwargs=kwargs)


class _FakeRouter:
    def __init__(self, rows: list[dict[str, Json]], recorder: _Recorder) -> None:
        self.rows = rows
        self.recorder = recorder

    def get_configured_token_limits(self, model_name: str) -> tuple[int | None, int | None]:
        for row in self.rows:
            if row.get("model_name") == model_name:
                limit = row.get("max_input_tokens")
                return (cast(int | None, limit), None)
        return (None, None)

    def embedding(self, **kwargs: Json) -> dict[str, Json]:
        return self.recorder.call(routed=True, kwargs=kwargs)

    async def aembedding(self, **kwargs: Json) -> dict[str, Json]:
        return self.recorder.call(routed=True, kwargs=kwargs)


def uninitialized_qdrant(
    embedding_model: str, max_input_tokens: int | None, timeout: float | None
) -> QdrantSemanticCache:
    cache = object.__new__(QdrantSemanticCache)
    cache.embedding_model = embedding_model
    cache.embedding_max_input_tokens = max_input_tokens
    cache.embedding_timeout = resolve_embedding_timeout(timeout)
    return cache


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def array_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def integer_value(value: Json, name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"{name} must be an integer")
    return value


def optional_integer(value: Json, name: str) -> int | None:
    if value is None:
        return None
    return integer_value(value, name)


def optional_number(value: Json, name: str) -> float | None:
    if value is None:
        return None
    if not isinstance(value, float):
        raise ValueError(f"{name} must be a number")
    return value


def boolean_value(value: Json, name: str) -> bool:
    if not isinstance(value, bool):
        raise ValueError(f"{name} must be a boolean")
    return value


def model_rows(input_value: dict[str, Json]) -> list[dict[str, Json]] | None:
    value = input_value.get("model_list")
    if value is None:
        return None
    return [object_value(row, "model_list entry") for row in array_value(value, "model_list")]


def vector_rows(input_value: dict[str, Json]) -> list[list[float]]:
    rows = array_value(input_value.get("vectors"), "vectors")
    return [
        [float(item) for item in cast(list[float], array_value(row, "vector"))]
        for row in rows
    ]


def router_model_list(rows: list[dict[str, Json]] | None) -> list[dict[str, Json]] | None:
    if rows is None:
        return None
    return [
        {
            "model_name": row.get("model_name"),
            "model_info": {"max_input_tokens": row.get("max_input_tokens")},
        }
        for row in rows
    ]


def backend_result(
    cache: _CacheEmbeddings,
    method: str,
    prompt: str,
    metadata: dict[str, object] | None,
    recorder: _Recorder,
) -> Json:
    recorder.calls.clear()
    try:
        if method == "async":
            call = cache._get_async_embedding  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            value = asyncio.run(call(prompt, metadata=metadata))
        else:
            call = cache._get_embedding  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            value = call(prompt, metadata=metadata)
    except Exception as error:  # noqa: BLE001
        return {"call": first_call(recorder), "outcome": "error", "status": status_code(error)}
    return {"call": first_call(recorder), "outcome": "ok", "value": cast(Json, value)}


def first_call(recorder: _Recorder) -> Json:
    return recorder.calls[0] if recorder.calls else None


def status_code(error: Exception) -> int | None:
    carried = getattr(error, "status_code", None)
    if isinstance(carried, int) and not isinstance(carried, bool):
        return carried
    return None


def run(input_value: dict[str, Json]) -> Json:
    embedding_model = string_value(input_value.get("embedding_model"), "embedding_model")
    prompt = string_value(input_value.get("prompt"), "prompt")
    caller = input_value.get("metadata")
    metadata: dict[str, object] | None = (
        None if caller is None else cast(dict[str, object], object_value(caller, "metadata"))
    )
    max_input_tokens = optional_integer(
        input_value.get("embedding_max_input_tokens"), "embedding_max_input_tokens"
    )
    timeout = optional_number(input_value.get("embedding_timeout"), "embedding_timeout")
    router_present = boolean_value(input_value.get("router_present"), "router_present")
    rows = model_rows(input_value)
    method = string_value(input_value.get("method"), "method")
    failing = string_value(input_value.get("outcome"), "outcome") == "failure"
    recorder = _Recorder(vector_rows(input_value), embedding_model, failing=failing)
    fake_router = _FakeRouter(rows or [], recorder) if router_present else None
    redis_cache = RedisSemanticCache(
        similarity_threshold=0.25,
        redis_url="redis://localhost:6379",
        embedding_model=embedding_model,
        embedding_max_input_tokens=max_input_tokens,
        embedding_timeout=timeout,
    )
    qdrant_cache = uninitialized_qdrant(embedding_model, max_input_tokens, timeout)
    with (
        patch("litellm.proxy.proxy_server.llm_router", fake_router),
        patch("litellm.proxy.proxy_server.llm_model_list", router_model_list(rows)),
        patch("litellm.embedding", recorder.direct_embedding),
        patch("litellm.aembedding", recorder.direct_aembedding),
    ):
        return {
            "redis": backend_result(redis_cache, method, prompt, metadata, recorder),
            "qdrant": backend_result(qdrant_cache, method, prompt, metadata, recorder),
        }
