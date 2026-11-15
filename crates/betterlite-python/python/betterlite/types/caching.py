from collections.abc import Sequence
from enum import StrEnum
from typing import Final, Literal

from pydantic import BaseModel, JsonValue
from typing_extensions import ReadOnly, TypedDict


class LiteLLMCacheType(StrEnum):
    LOCAL = "local"
    REDIS = "redis"
    REDIS_SEMANTIC = "redis-semantic"
    VALKEY_SEMANTIC = "valkey-semantic"
    S3 = "s3"
    DISK = "disk"
    QDRANT_SEMANTIC = "qdrant-semantic"
    AZURE_BLOB = "azure-blob"
    GCS = "gcs"


class SemanticCacheScope(StrEnum):
    KEY = "key"
    END_USER = "end_user"


CachingSupportedCallTypes = Literal[
    "completion",
    "acompletion",
    "embedding",
    "aembedding",
    "atranscription",
    "transcription",
    "atext_completion",
    "text_completion",
    "arerank",
    "rerank",
    "responses",
    "aresponses",
    "anthropic_messages",
    "aanthropic_messages",
]

# ast-grep-ignore: no-long-names
DEFAULT_CACHING_SUPPORTED_CALL_TYPES: tuple[CachingSupportedCallTypes, ...] = (
    "completion",
    "acompletion",
    "embedding",
    "aembedding",
    "atranscription",
    "transcription",
    "atext_completion",
    "text_completion",
    "arerank",
    "rerank",
    "responses",
    "aresponses",
    "anthropic_messages",
    "aanthropic_messages",
)


class RedisPipelineIncrementOperation(TypedDict):  # ast-grep-ignore: no-long-names
    key: str
    increment_value: float
    ttl: int | None


class RedisPipelineSetOperation(TypedDict):
    key: str
    value: JsonValue
    ttl: int | None


class RedisPipelineRpushOperation(TypedDict):
    key: str
    values: Sequence[JsonValue]


class RedisPipelineLpopOperation(TypedDict):
    key: str
    count: int | None


DynamicCacheControl = TypedDict(
    "DynamicCacheControl",
    {
        # Will cache the response for the user-defined amount of time (in seconds).
        "ttl": int | None,
        # Namespace to use for caching
        "namespace": str | None,
        # Max Age to use for caching
        "s-maxage": int | None,
        "s-max-age": int | None,
        # Will not return a cached response, but instead call the actual endpoint.
        "no-cache": bool | None,
        # Will not store the response in the cache.
        "no-store": bool | None,
    },
)


class CachePingResponse(BaseModel):
    status: str
    cache_type: str
    ping_response: bool | None = None
    set_cache_response: str | None = None
    litellm_cache_params: str | None = None
    health_check_cache_params: dict[str, JsonValue] | None = None


class HealthCheckCacheParams(BaseModel):
    host: str | None = None
    port: str | int | None = None
    redis_kwargs: dict[str, JsonValue] | None = None
    namespace: str | None = None
    redis_version: str | int | float | None = None


EMBEDDING_CACHE_FORMAT_VERSION: Final = 2


class CachedEmbedding(TypedDict):
    embedding: ReadOnly[list[float] | str | None]
    index: ReadOnly[int | None]
    object: ReadOnly[str | None]
    model: ReadOnly[str | None]
    prompt_tokens: ReadOnly[int | None]
    prompt_tokens_details: ReadOnly[dict[str, JsonValue] | None]
    format_version: ReadOnly[int]
