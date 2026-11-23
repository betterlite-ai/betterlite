import asyncio
import importlib
import logging
from collections.abc import Iterator
from typing import Final, Protocol

import pytest
from pydantic import JsonValue
from typing_extensions import override

from betterlite._cache_ops import CacheOperations
from betterlite._cache_ops.types import CacheStore
from betterlite._native import NativeCache
from betterlite.types.caching import LiteLLMCacheType
from betterlite.types.utils.base import Dumpable, InitValue

CHAT: Final[dict[str, JsonValue]] = {
    "model": "gpt-4o",
    "messages": [{"role": "user", "content": "hello"}],
}
CHAT_RESPONSE: Final[dict[str, JsonValue]] = {"id": "chatcmpl-1", "content": "hello"}
EMBEDDING: Final[dict[str, JsonValue]] = {
    "model": "text-embedding-3-small",
    "input": ["a", "b"],
}
EMBEDDING_RESPONSE: Final[dict[str, JsonValue]] = {
    "model": "text-embedding-3-small",
    "data": [
        {"embedding": [0.5, 0.25], "index": 0, "object": "embedding"},
        {"embedding": [0.75, 1.0], "index": 1, "object": "embedding"},
    ],
    "usage": {"prompt_tokens": 7, "total_tokens": 7},
}
FILE: Final[dict[str, JsonValue]] = {
    "model": "whisper-1",
    "file": "audio.mp3",
    "metadata": {"file_checksum": "checksum-1"},
}
NESTED_NAMESPACE: Final[dict[str, JsonValue]] = {
    "model": "gpt-4o",
    "messages": [{"role": "user", "content": "hello"}],
    "metadata": {"redis_namespace": "tenant"},
}
REQUEST_NAMESPACE: Final[dict[str, JsonValue]] = {
    "model": "gpt-4o",
    "messages": [{"role": "user", "content": "hello"}],
    "cache": {"namespace": "request"},
}
GROUPS: Final[dict[str, JsonValue]] = {
    "model": "gpt-4o",
    "messages": [{"role": "user", "content": "hello"}],
    "metadata": {"model_group": "chat", "caching_groups": [["chat", "backup"]]},
}
PRESET: Final[dict[str, JsonValue]] = {
    "model": "gpt-4o",
    "messages": [{"role": "user", "content": "hello"}],
    "litellm_params": {"preset_cache_key": "given"},
}
EMBEDDING_WRITES: Final = 2
STREAM_TEXT: Final = "abcdefghijkl"
STREAM_CHUNKS: Final[list[dict[str, JsonValue]]] = [
    {"choices": [{"delta": {"role": "assistant", "content": "abcde"}}]},
    {"choices": [{"delta": {"role": "assistant", "content": "fghij"}}]},
    {"choices": [{"delta": {"role": "assistant", "content": "kl"}}]},
]
FAILURE_TEXT: Final = "backend is down"


class LiteLLMCache(Protocol):
    def get_cache_key(self, **kwargs: JsonValue) -> str: ...
    def get_cache(
        self, dynamic_cache_object: CacheStore | None = None, **kwargs: JsonValue
    ) -> JsonValue: ...
    async def async_get_cache(
        self, dynamic_cache_object: CacheStore | None = None, **kwargs: JsonValue
    ) -> JsonValue: ...
    def add_cache(self, result: InitValue, **kwargs: JsonValue) -> None: ...
    async def async_add_cache(
        self,
        result: InitValue,
        dynamic_cache_object: CacheStore | None = None,
        **kwargs: JsonValue,
    ) -> None: ...
    def add_embedding_response_to_cache(  # ast-grep-ignore: no-long-names
        self,
        result: InitValue,
        input_value: JsonValue,
        kwargs: dict[str, JsonValue],
        idx_in_result_data: int = 0,
    ) -> tuple[str, dict[str, JsonValue], dict[str, JsonValue]]: ...
    async def async_add_cache_pipeline(
        self,
        result: InitValue,
        dynamic_cache_object: CacheStore | None = None,
        **kwargs: JsonValue,
    ) -> None: ...
    async def batch_cache_write(self, result: InitValue, **kwargs: JsonValue) -> None: ...
    def should_use_cache(self, **kwargs: JsonValue) -> bool: ...
    def generate_streaming_content(self, content: str) -> Iterator[dict[str, JsonValue]]: ...
    async def ping(self) -> bool | None: ...
    async def delete_cache_keys(self, keys: list[str]) -> None: ...
    async def disconnect(self) -> None: ...


class CacheFactory(Protocol):
    def __call__(self, **kwargs: JsonValue | CacheStore) -> LiteLLMCache: ...


class EmbeddingFactory(Protocol):
    def __call__(self, **kwargs: JsonValue) -> Dumpable: ...


class CacheModule(Protocol):
    def __getattr__(self, name: str) -> CacheFactory: ...


class EmbeddingModule(Protocol):
    def __getattr__(self, name: str) -> EmbeddingFactory: ...


LITELLM_CACHE: CacheModule = importlib.import_module("litellm")
LITELLM_EMBEDDING: EmbeddingModule = importlib.import_module("litellm")


class RecordingBackend:
    def __init__(self) -> None:
        self.values: dict[str, JsonValue] = {}
        self.calls: list[tuple[str, str]] = []
        self.arguments: list[dict[str, JsonValue]] = []
        self.pinged = False
        self.deleted: list[str] = []
        self.disconnected = False

    def get_cache(self, key: str, **kwargs: JsonValue) -> JsonValue:
        self.calls.append(("get_cache", key))
        self.arguments.append(dict(kwargs))
        return self.values.get(key)

    def set_cache(self, key: str, value: JsonValue, **_kwargs: JsonValue) -> None:
        self.calls.append(("set_cache", key))
        self.values[key] = value

    async def async_get_cache(self, key: str, **kwargs: JsonValue) -> JsonValue:
        self.calls.append(("async_get_cache", key))
        self.arguments.append(dict(kwargs))
        return self.values.get(key)

    async def async_set_cache(self, key: str, value: JsonValue, **_kwargs: JsonValue) -> None:
        self.calls.append(("async_set_cache", key))
        self.values[key] = value

    async def async_set_cache_pipeline(
        self, cache_list: list[tuple[str, JsonValue]], **_kwargs: JsonValue
    ) -> None:
        self.calls.append(("async_set_cache_pipeline", ""))
        for key, value in cache_list:
            self.values[key] = value

    async def batch_cache_write(self, key: str, value: JsonValue, **_kwargs: JsonValue) -> None:
        self.calls.append(("batch_cache_write", key))
        self.values[key] = value

    async def ping(self) -> bool:
        self.pinged = True
        return True

    # ast-grep-ignore: no-one-line-functions
    async def delete_cache_keys(self, keys: list[str]) -> None:
        self.deleted.extend(keys)

    # ast-grep-ignore: no-one-line-functions
    async def disconnect(self) -> None:
        self.disconnected = True


class FailingBackend(RecordingBackend):
    @override
    def set_cache(self, _key: str, _value: JsonValue, **_kwargs: JsonValue) -> None:
        raise RuntimeError(FAILURE_TEXT)


class FailingReadBackend(RecordingBackend):
    @override
    def get_cache(self, _key: str, **_kwargs: JsonValue) -> JsonValue:
        raise RuntimeError(FAILURE_TEXT)


class Cache(CacheOperations):
    def __init__(
        self,
        backend: CacheStore,
        *,
        namespace: str | None = None,
        ttl: float | None = None,
        mode: str = "default_on",
        native: NativeCache | None = None,
    ) -> None:
        self.cache = backend
        self.type = LiteLLMCacheType.LOCAL
        self.namespace = namespace
        self.ttl = ttl
        self.mode = mode
        self.supported_call_types = None
        self.redis_flush_size = None
        self.semantic_cache_scope = "key"
        self._native_cache = native


def matched_caches(
    ours_backend: CacheStore,
    reference_backend: CacheStore,
    *,
    namespace: str | None = None,
    ttl: float | None = None,
    mode: str = "default_on",
) -> tuple[Cache, LiteLLMCache]:
    ours = Cache(ours_backend, namespace=namespace, ttl=ttl, mode=mode)
    reference = LITELLM_CACHE.Cache(
        _backend=reference_backend, namespace=namespace, ttl=ttl, mode=mode, type="local"
    )
    return ours, reference


def test_cache_ops_chat_embed_keys() -> None:
    ours, reference = matched_caches(RecordingBackend(), RecordingBackend())
    assert ours.get_cache_key(**CHAT) == reference.get_cache_key(**CHAT)
    assert ours.get_cache_key(**EMBEDDING) == reference.get_cache_key(**EMBEDDING)


def test_cache_ops_file_ns_keys() -> None:
    ours, reference = matched_caches(RecordingBackend(), RecordingBackend())
    assert ours.get_cache_key(**FILE) == reference.get_cache_key(**FILE)
    assert ours.get_cache_key(**NESTED_NAMESPACE) == reference.get_cache_key(**NESTED_NAMESPACE)
    assert ours.get_cache_key(**REQUEST_NAMESPACE) == reference.get_cache_key(**REQUEST_NAMESPACE)
    named, named_reference = matched_caches(
        RecordingBackend(), RecordingBackend(), namespace="tenant"
    )
    assert named.get_cache_key(**CHAT) == named_reference.get_cache_key(**CHAT)


def test_cache_ops_group_keys() -> None:
    ours, reference = matched_caches(RecordingBackend(), RecordingBackend())
    assert ours.get_cache_key(**GROUPS) == reference.get_cache_key(**GROUPS)


def test_cache_ops_preset_key() -> None:
    ours, reference = matched_caches(RecordingBackend(), RecordingBackend())
    assert ours.get_cache_key(**PRESET) == reference.get_cache_key(**PRESET) == "given"


def test_cache_ops_native_key() -> None:
    native = NativeCache('{"type": "local"}')
    ours = Cache(RecordingBackend(), native=native)
    reference = LITELLM_CACHE.Cache(_backend=RecordingBackend(), type="local")
    assert ours.get_cache_key(**CHAT) == reference.get_cache_key(**CHAT)


def test_cache_ops_lookup_miss() -> None:
    ours, reference = matched_caches(RecordingBackend(), RecordingBackend())
    assert ours.get_cache(None, **CHAT) is None
    assert reference.get_cache(None, **CHAT) is None
    failing = Cache(FailingReadBackend())
    assert failing.get_cache(None, **CHAT) is None
    failing_reference = LITELLM_CACHE.Cache(_backend=FailingReadBackend(), type="local")
    assert failing_reference.get_cache(None, **CHAT) is None


def test_cache_ops_lookup_hit() -> None:
    ours_backend = RecordingBackend()
    reference_backend = RecordingBackend()
    ours, reference = matched_caches(ours_backend, reference_backend)
    ours.add_cache(CHAT_RESPONSE, None, **CHAT)
    reference.add_cache(CHAT_RESPONSE, **CHAT)
    assert ours.get_cache(None, **CHAT) == reference.get_cache(None, **CHAT) == CHAT_RESPONSE


def test_cache_ops_lookup_stale() -> None:
    ours, reference = matched_caches(RecordingBackend(), RecordingBackend())
    ours.add_cache(CHAT_RESPONSE, None, **CHAT)
    reference.add_cache(CHAT_RESPONSE, **CHAT)
    stale: dict[str, JsonValue] = {**CHAT, "cache": {"s-maxage": -1}}
    fresh: dict[str, JsonValue] = {**CHAT, "cache": {"s-maxage": 1000}}
    assert ours.get_cache(None, **stale) is None
    assert reference.get_cache(None, **stale) is None
    assert ours.get_cache(None, **fresh) == reference.get_cache(None, **fresh) == CHAT_RESPONSE


def test_cache_ops_lookup_dynamic() -> None:
    ours_backend = RecordingBackend()
    reference_backend = RecordingBackend()
    ours, reference = matched_caches(ours_backend, reference_backend)
    ours.add_cache(CHAT_RESPONSE, None, **CHAT)
    reference.add_cache(CHAT_RESPONSE, **CHAT)
    ours_dynamic = RecordingBackend()
    reference_dynamic = RecordingBackend()
    assert ours.get_cache(ours_dynamic, **CHAT) is None
    assert reference.get_cache(reference_dynamic, **CHAT) is None
    assert [name for name, _key in ours_backend.calls] == ["set_cache"]
    assert [name for name, _key in ours_dynamic.calls] == ["get_cache"]
    assert ours_dynamic.calls == reference_dynamic.calls
    assert ours_dynamic.arguments == reference_dynamic.arguments
    ours.add_cache(CHAT_RESPONSE, ours_dynamic, **CHAT)
    assert ours.get_cache(ours_dynamic, **CHAT) == CHAT_RESPONSE
    ours_async = RecordingBackend()
    reference_async = RecordingBackend()
    assert asyncio.run(ours.async_get_cache(ours_async, **CHAT)) is None
    assert asyncio.run(reference.async_get_cache(reference_async, **CHAT)) is None
    assert ours_async.arguments == reference_async.arguments


def test_cache_ops_async_lookup() -> None:
    ours, reference = matched_caches(RecordingBackend(), RecordingBackend())
    ours.add_cache(CHAT_RESPONSE, None, **CHAT)
    reference.add_cache(CHAT_RESPONSE, **CHAT)
    assert asyncio.run(ours.async_get_cache(None, **CHAT)) == CHAT_RESPONSE
    assert asyncio.run(reference.async_get_cache(None, **CHAT)) == CHAT_RESPONSE


def test_cache_ops_should_use() -> None:
    off, off_reference = matched_caches(RecordingBackend(), RecordingBackend(), mode="default_off")
    assert off.should_use_cache(**CHAT) is False
    assert off_reference.should_use_cache(**CHAT) is False
    opted_in: dict[str, JsonValue] = {**CHAT, "cache": {"use-cache": True}}
    assert off.should_use_cache(**opted_in) is True
    assert off_reference.should_use_cache(**opted_in) is True
    opted_out: dict[str, JsonValue] = {**CHAT, "cache": {"use-cache": False}}
    assert off.should_use_cache(**opted_out) is False
    assert off_reference.should_use_cache(**opted_out) is False
    on, on_reference = matched_caches(RecordingBackend(), RecordingBackend())
    assert on.should_use_cache(**CHAT) is True
    assert on_reference.should_use_cache(**CHAT) is True


def test_cache_ops_write_entries() -> None:
    ours_backend = RecordingBackend()
    reference_backend = RecordingBackend()
    ours, reference = matched_caches(ours_backend, reference_backend)
    ours.add_cache(CHAT_RESPONSE, None, **CHAT)
    reference.add_cache(CHAT_RESPONSE, **CHAT)
    assert sorted(ours_backend.values) == sorted(reference_backend.values)
    asyncio.run(ours.async_add_cache(CHAT_RESPONSE, None, **CHAT))
    asyncio.run(reference.async_add_cache(CHAT_RESPONSE, None, **CHAT))
    assert sorted(ours_backend.values) == sorted(reference_backend.values)
    assert ours.get_cache(None, **CHAT) == reference.get_cache(None, **CHAT) == CHAT_RESPONSE
    off, off_reference = matched_caches(RecordingBackend(), RecordingBackend(), mode="default_off")
    off.add_cache(CHAT_RESPONSE, None, **CHAT)
    off_reference.add_cache(CHAT_RESPONSE, **CHAT)
    assert off.get_cache(None, **CHAT) is None
    assert off_reference.get_cache(None, **CHAT) is None


def test_cache_ops_write_failure(caplog: pytest.LogCaptureFixture) -> None:
    cache = Cache(FailingBackend())
    with caplog.at_level(logging.ERROR, logger="LiteLLM"):
        cache.add_cache(CHAT_RESPONSE, None, **CHAT)
        asyncio.run(cache.async_add_cache(CHAT_RESPONSE, None, **CHAT))
    assert "LiteLLM Cache: exception in add_cache" in caplog.text
    assert FAILURE_TEXT in caplog.text
    reference = LITELLM_CACHE.Cache(_backend=FailingBackend(), type="local")
    reference.add_cache(CHAT_RESPONSE, **CHAT)


def test_cache_ops_embed_keys() -> None:
    ours, reference = matched_caches(RecordingBackend(), RecordingBackend())
    embedding = LITELLM_EMBEDDING.EmbeddingResponse(**EMBEDDING_RESPONSE)
    ours_key, ours_data, _ = ours.add_embedding_response_to_cache(
        embedding, "a", dict(EMBEDDING), 0
    )
    reference_key, reference_data, _ = reference.add_embedding_response_to_cache(
        embedding, "a", dict(EMBEDDING), 0
    )
    assert ours_key == reference_key
    assert ours_data["response"] == reference_data["response"]


def test_cache_ops_pipeline_write() -> None:
    ours_backend = RecordingBackend()
    reference_backend = RecordingBackend()
    ours, reference = matched_caches(ours_backend, reference_backend)
    embedding = LITELLM_EMBEDDING.EmbeddingResponse(**EMBEDDING_RESPONSE)
    asyncio.run(ours.async_add_cache_pipeline(embedding, None, **EMBEDDING))
    asyncio.run(reference.async_add_cache_pipeline(embedding, None, **EMBEDDING))
    assert sorted(ours_backend.values) == sorted(reference_backend.values)
    assert len(ours_backend.values) == EMBEDDING_WRITES
    first = {**EMBEDDING, "input": "a"}
    second = {**EMBEDDING, "input": "b"}
    missing = {**EMBEDDING, "input": "c"}
    ours_first = ours.get_cache(None, **first)
    reference_first = reference.get_cache(None, **first)
    assert ours_first is not None
    assert ours_first == reference_first
    assert ours.get_cache(None, **second) == reference.get_cache(None, **second)
    assert ours.get_cache(None, **missing) is None
    assert reference.get_cache(None, **missing) is None


def test_cache_ops_batch_write() -> None:
    ours_backend = RecordingBackend()
    reference_backend = RecordingBackend()
    ours, reference = matched_caches(ours_backend, reference_backend)
    asyncio.run(ours.batch_cache_write(CHAT_RESPONSE, **CHAT))
    asyncio.run(reference.batch_cache_write(CHAT_RESPONSE, **CHAT))
    assert sorted(ours_backend.values) == sorted(reference_backend.values)
    assert ours.get_cache(None, **CHAT) == reference.get_cache(None, **CHAT) == CHAT_RESPONSE


def test_cache_ops_streaming() -> None:
    ours, reference = matched_caches(RecordingBackend(), RecordingBackend())
    assert list(ours.generate_streaming_content(STREAM_TEXT)) == STREAM_CHUNKS
    assert list(reference.generate_streaming_content(STREAM_TEXT)) == STREAM_CHUNKS


def test_cache_ops_backend_calls() -> None:
    backend = RecordingBackend()
    cache = Cache(backend)
    assert asyncio.run(cache.ping()) is True
    asyncio.run(cache.delete_cache_keys(["a", "b"]))
    asyncio.run(cache.disconnect())
    assert backend.pinged is True
    assert backend.deleted == ["a", "b"]
    assert backend.disconnected is True
    reference_backend = RecordingBackend()
    reference = LITELLM_CACHE.Cache(_backend=reference_backend, type="local")
    assert asyncio.run(reference.ping()) is True
    asyncio.run(reference.delete_cache_keys(["a", "b"]))
    asyncio.run(reference.disconnect())
    assert reference_backend.pinged is True
    assert reference_backend.deleted == ["a", "b"]
    assert reference_backend.disconnected is True


def test_cache_ops_native_write() -> None:
    ours = Cache(RecordingBackend(), native=NativeCache('{"type": "local"}'))
    reference = LITELLM_CACHE.Cache(_backend=RecordingBackend(), type="local")
    assert ours.get_cache(None, **CHAT) is None
    assert reference.get_cache(None, **CHAT) is None
    ours.add_cache(CHAT_RESPONSE, None, **CHAT)
    reference.add_cache(CHAT_RESPONSE, **CHAT)
    assert ours.get_cache(None, **CHAT) == reference.get_cache(None, **CHAT) == CHAT_RESPONSE
