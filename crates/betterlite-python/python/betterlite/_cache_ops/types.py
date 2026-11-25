from typing import Protocol, runtime_checkable

from pydantic import JsonValue
from typing_extensions import TypedDict


# The BaseCache methods every backend answers, the ones a dynamic_cache_object has too.
@runtime_checkable
class CacheStore(Protocol):
    def get_cache(self, key: str, **kwargs: JsonValue) -> JsonValue: ...

    def set_cache(self, key: str, value: JsonValue, **kwargs: JsonValue) -> None: ...

    async def async_get_cache(self, key: str, **kwargs: JsonValue) -> JsonValue: ...

    async def async_set_cache(
        self, key: str, value: JsonValue, **kwargs: JsonValue
    ) -> bool | None: ...

    async def async_set_cache_pipeline(
        self, cache_list: list[tuple[str, JsonValue]], **kwargs: JsonValue
    ) -> None: ...

    async def batch_cache_write(self, key: str, value: JsonValue, **kwargs: JsonValue) -> None: ...


# LiteLLM reaches ping and delete_cache_keys only on a backend that declares them, like RedisCache.
@runtime_checkable
class CacheLifecycle(CacheStore, Protocol):
    async def ping(self) -> bool: ...

    async def delete_cache_keys(self, keys: list[str]) -> None: ...

    async def disconnect(self) -> None: ...


class CacheSettings(TypedDict, total=False):
    type: str
    mode: str
    namespace: str
    ttl: float
    semantic_cache_scope: str
    include_provider_params: bool


# What a Rust write answers: the key, the envelope a backend stores, and the ttl the write applies.
class StoredAnswer(TypedDict):
    cache_key: str
    cached_data: dict[str, JsonValue]
    ttl: float | None
