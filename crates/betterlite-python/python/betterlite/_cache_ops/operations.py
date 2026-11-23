import json
import logging
import os
import time
from collections.abc import Iterator
from typing import Final

import betterlite
from betterlite._cache_ops import wire
from betterlite._cache_ops.types import CacheLifecycle, CacheSettings, CacheStore, StoredAnswer
from betterlite._native import NativeCache, cache_decode, cache_embedding, cache_envelope, cache_key
from betterlite.types.caching import CachingSupportedCallTypes, LiteLLMCacheType
from betterlite.types.utils.base import InitValue
from pydantic import JsonValue

_LOGGER: Final = logging.getLogger("LiteLLM")
_CHUNK_SIZE: Final = 5
_SEMANTIC_TYPES: Final[frozenset[str]] = frozenset(
    {"redis-semantic", "valkey-semantic", "qdrant-semantic"}
)
_DELAY_TEXT: Final = os.environ.get("CACHED_STREAMING_CHUNK_DELAY")
# LiteLLM's CACHED_STREAMING_CHUNK_DELAY, which an empty value leaves at the default
CACHED_STREAMING_CHUNK_DELAY: Final[float] = (
    0.02 if _DELAY_TEXT is None or _DELAY_TEXT == "" else float(_DELAY_TEXT)
)


class CacheOperations:
    cache: CacheStore
    _native_cache: NativeCache | None = None
    type: LiteLLMCacheType
    namespace: str | None
    mode: str
    ttl: float | None
    supported_call_types: list[CachingSupportedCallTypes] | None
    redis_flush_size: int | None
    semantic_cache_scope: str

    def get_cache_key(self, **kwargs: JsonValue) -> str:
        return self._derive(wire.text_of({"kwargs": wire.key_arguments(kwargs)}))

    def should_use_cache(self, **kwargs: JsonValue) -> bool:
        if self.mode == "default_on":
            return True
        match kwargs.get("cache"):
            case dict() as control:
                return control.get("use-cache") is True
            case None | str() | int() | float() | bool() | list():
                return False

    def get_cache(
        self, dynamic_cache_object: CacheStore | None = None, **kwargs: JsonValue
    ) -> JsonValue:
        try:
            if not self.should_use_cache(**kwargs):
                return None
            return self._read(dynamic_cache_object, kwargs)
        except Exception:
            _LOGGER.debug("An exception occurred", exc_info=True)
            return None

    async def async_get_cache(
        self, dynamic_cache_object: CacheStore | None = None, **kwargs: JsonValue
    ) -> JsonValue:
        try:
            if not self.should_use_cache(**kwargs):
                return None
            return await self._async_read(dynamic_cache_object, kwargs)
        except Exception:
            _LOGGER.debug("An exception occurred", exc_info=True)
            return None

    def add_cache(
        self,
        result: InitValue,
        dynamic_cache_object: CacheStore | None = None,
        **kwargs: JsonValue,
    ) -> None:
        try:
            if not self.should_use_cache(**kwargs):
                return
            self._write(result, dynamic_cache_object, kwargs)
        except Exception as error:
            # LiteLLM's _log_add_cache_failure inlined: ruff's BLE001 exempts a logging handler.
            _LOGGER.exception("LiteLLM Cache: exception in add_cache: %s", error)

    async def async_add_cache(
        self,
        result: InitValue,
        dynamic_cache_object: CacheStore | None = None,
        **kwargs: JsonValue,
    ) -> None:
        try:
            if not self.should_use_cache(**kwargs):
                return
            if self.type == "redis" and self.redis_flush_size is not None:
                await self.batch_cache_write(result, **kwargs)
                return
            await self._async_write(result, dynamic_cache_object, kwargs)
        except Exception as error:
            _LOGGER.exception("LiteLLM Cache: exception in add_cache: %s", error)

    def add_embedding_response_to_cache(
        self,
        result: InitValue,
        input_value: JsonValue,
        kwargs: dict[str, JsonValue],
        idx_in_result_data: int = 0,
    ) -> tuple[str, dict[str, JsonValue], dict[str, JsonValue]]:
        answer = self._entry(kwargs, result, input_value, idx_in_result_data)
        return answer["cache_key"], answer["cached_data"], wire.stored_arguments(kwargs, answer)

    async def async_add_cache_pipeline(
        self,
        result: InitValue,
        dynamic_cache_object: CacheStore | None = None,
        **kwargs: JsonValue,
    ) -> None:
        try:
            if not self.should_use_cache(**kwargs):
                return
            await self._pipeline(result, dynamic_cache_object, kwargs)
        except Exception as error:
            _LOGGER.exception("LiteLLM Cache: exception in add_cache: %s", error)

    async def batch_cache_write(self, result: InitValue, **kwargs: JsonValue) -> None:
        key, cached_data, stored = self._add_cache_logic(result, kwargs)
        await self.cache.batch_cache_write(key, cached_data, **stored)

    def generate_streaming_content(self, content: str) -> Iterator[dict[str, JsonValue]]:
        for index in range(0, len(content), _CHUNK_SIZE):
            text = content[index : index + _CHUNK_SIZE]
            yield {"choices": [{"delta": {"role": "assistant", "content": text}}]}
            time.sleep(CACHED_STREAMING_CHUNK_DELAY)

    async def ping(self) -> bool | None:
        match self.cache:
            case CacheLifecycle() as cache:
                return await cache.ping()
        return None

    async def delete_cache_keys(self, keys: list[str]) -> None:
        match self.cache:
            case CacheLifecycle() as cache:
                await cache.delete_cache_keys(keys)

    async def disconnect(self) -> None:
        match self.cache:
            case CacheLifecycle() as cache:
                await cache.disconnect()

    def _read(self, dynamic: CacheStore | None, kwargs: dict[str, JsonValue]) -> JsonValue:
        native = self._native_cache
        if dynamic is not None or native is None:
            return self._backend_read(dynamic, kwargs)
        request = wire.text_of({"kwargs": kwargs})
        if not self._is_semantic_cache():
            return wire.value_of(native.lookup(request))
        response, similarity = wire.semantic_answer(native.lookup_semantic(request))
        wire.stamp_similarity(kwargs, similarity)
        return response

    async def _async_read(
        self, dynamic: CacheStore | None, kwargs: dict[str, JsonValue]
    ) -> JsonValue:
        native = self._native_cache
        if dynamic is not None or native is None:
            # An async backend lookup gets every kwarg; the sync one gets the prompt and metadata.
            backend = self.cache if dynamic is None else dynamic
            key = self._derive(wire.text_of({"kwargs": kwargs}))
            return self._get_cache_logic(await backend.async_get_cache(key, **kwargs), kwargs)
        request = wire.text_of({"kwargs": kwargs})
        if not self._is_semantic_cache():
            return wire.value_of(await native.async_lookup(request))
        response, similarity = wire.semantic_answer(await native.async_lookup_semantic(request))
        wire.stamp_similarity(kwargs, similarity)
        return response

    def _backend_read(
        self, dynamic: CacheStore | None, kwargs: dict[str, JsonValue]
    ) -> JsonValue:
        backend = self.cache if dynamic is None else dynamic
        lookup = wire.lookup_arguments(kwargs)
        cached = backend.get_cache(self._derive(wire.text_of({"kwargs": kwargs})), **lookup)
        wire.carry_similarity(kwargs, lookup)
        return self._get_cache_logic(cached, kwargs)

    def _write(
        self, result: InitValue, dynamic: CacheStore | None, kwargs: dict[str, JsonValue]
    ) -> None:
        native = self._native_cache
        if native is not None and dynamic is None:
            native.write(wire.write_call(kwargs, result))
            return
        key, cached_data, stored = self._add_cache_logic(result, kwargs)
        backend = self.cache if dynamic is None else dynamic
        backend.set_cache(key, cached_data, **stored)

    async def _async_write(
        self, result: InitValue, dynamic: CacheStore | None, kwargs: dict[str, JsonValue]
    ) -> None:
        native = self._native_cache
        if native is not None and dynamic is None:
            await native.async_write(wire.write_call(kwargs, result))
            return
        key, cached_data, stored = self._add_cache_logic(result, kwargs)
        backend = self.cache if dynamic is None else dynamic
        await backend.async_set_cache(key, cached_data, **stored)

    async def _pipeline(
        self, result: InitValue, dynamic: CacheStore | None, kwargs: dict[str, JsonValue]
    ) -> None:
        native = self._native_cache
        if native is not None and dynamic is None:
            await native.async_write_pipeline(wire.write_call(kwargs, result))
            return
        count, inputs = wire.embedding_inputs(kwargs)
        records = wire.embedding_records(result)
        if count != len(records):
            _LOGGER.debug(
                "LiteLLM Cache: skipping embedding cache write, %d inputs but %d embeddings"
                " in the response",
                count,
                len(records),
            )
            return
        stored = dict(kwargs)
        if self.ttl is not None:
            stored["ttl"] = self.ttl
        entries: list[tuple[str, JsonValue]] = []
        for index, item in enumerate(inputs):
            key, cached_data, stored = self.add_embedding_response_to_cache(
                result, item, kwargs, index
            )
            entries.append((key, cached_data))
        backend = self.cache if dynamic is None else dynamic
        await backend.async_set_cache_pipeline(entries, **stored)

    # LiteLLM's _get_cache_logic: Rust checks the age against the cache control and decodes.
    def _get_cache_logic(self, cached: JsonValue, kwargs: dict[str, JsonValue]) -> JsonValue:
        return wire.value_of(cache_decode(wire.text_of({"kwargs": kwargs, "value": cached})))

    # LiteLLM's _add_cache_logic: the key, the envelope to store and the kwargs the write carries.
    def _add_cache_logic(
        self, result: InitValue, kwargs: dict[str, JsonValue]
    ) -> tuple[str, dict[str, JsonValue], dict[str, JsonValue]]:
        answer = self._envelope(kwargs, result)
        return answer["cache_key"], answer["cached_data"], wire.stored_arguments(kwargs, answer)

    def _envelope(self, kwargs: dict[str, JsonValue], result: InitValue) -> StoredAnswer:
        return wire.stored_of(
            cache_envelope(self._settings_text(), wire.write_call(kwargs, result))
        )

    def _entry(
        self, kwargs: dict[str, JsonValue], result: InitValue, item: JsonValue, index: int
    ) -> StoredAnswer:
        body: dict[str, JsonValue] = {
            "kwargs": kwargs, "result": wire.dumped(result), "input": item, "index": index,
        }
        native = self._native_cache
        if native is not None:
            return wire.stored_of(native.write_embedding(wire.text_of(body)))
        return wire.stored_of(cache_embedding(self._settings_text(), wire.text_of(body)))

    def _derive(self, request: str) -> str:
        native = self._native_cache
        if native is not None:
            return native.cache_key(request)
        return cache_key(self._settings_text(), request)

    def _settings_text(self) -> str:
        provider_params = betterlite.enable_caching_on_provider_specific_optional_params
        settings: CacheSettings = {
            "type": self.type,
            "mode": self.mode,
            "semantic_cache_scope": self.semantic_cache_scope,
            "include_provider_params": provider_params,
        }
        if self.namespace is not None:
            settings["namespace"] = self.namespace
        if self.ttl is not None:
            settings["ttl"] = self.ttl
        return json.dumps(settings)

    def _is_semantic_cache(self) -> bool:
        return self.type in _SEMANTIC_TYPES
