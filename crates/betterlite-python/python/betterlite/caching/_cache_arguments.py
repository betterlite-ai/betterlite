from typing import Final

from pydantic import ConfigDict, JsonValue, TypeAdapter
from typing_extensions import TypedDict

from betterlite.caching.base_cache import BaseCache


# The keywords enable_cache and update_cache hand on to Cache, apart from the ones they name themselves.
class CacheArguments(TypedDict, total=False):
    mode: str | None
    namespace: str | None
    ttl: float | None
    default_in_memory_ttl: float | None
    default_in_redis_ttl: float | None
    similarity_threshold: float | None
    azure_account_url: str | None
    azure_blob_container: str | None
    s3_bucket_name: str | None
    s3_region_name: str | None
    s3_api_version: str | None
    s3_use_ssl: bool | None
    s3_verify: bool | str | None
    s3_endpoint_url: str | None
    s3_aws_access_key_id: str | None
    s3_aws_secret_access_key: str | None
    s3_aws_session_token: str | None
    # ast-grep-ignore: no-loose-types - LiteLLM declares this parameter as object
    s3_config: object | None
    s3_path: str | None
    gcs_bucket_name: str | None
    gcs_path_service_account: str | None
    gcs_path: str | None
    # ast-grep-ignore: no-long-names
    redis_semantic_cache_embedding_model: str
    # ast-grep-ignore: no-long-names
    redis_semantic_cache_index_name: str | None
    # ast-grep-ignore: no-long-names
    valkey_semantic_cache_embedding_model: str
    # ast-grep-ignore: no-long-names
    valkey_semantic_cache_index_name: str | None
    redis_flush_size: int | None
    redis_startup_nodes: list[JsonValue] | None
    disk_cache_dir: str | None
    qdrant_api_base: str | None
    qdrant_api_key: str | None
    qdrant_collection_name: str | None
    qdrant_quantization_config: str | None
    # ast-grep-ignore: no-long-names
    qdrant_semantic_cache_embedding_model: str
    # ast-grep-ignore: no-long-names
    qdrant_semantic_cache_vector_size: int | None
    # ast-grep-ignore: no-long-names
    semantic_cache_embedding_max_input_tokens: int | None
    # ast-grep-ignore: no-long-names
    semantic_cache_embedding_timeout: float | None
    semantic_cache_scope: str
    gcp_service_account: str | None
    gcp_ssl_ca_certs: str | None
    _backend: BaseCache | None


# RedisCache names these two itself; any other keyword is a redis client option it keeps as given.
class RedisOverrides(TypedDict, total=False):
    startup_nodes: list[JsonValue] | None
    socket_timeout: float | None


REDIS_OVERRIDES: Final = TypeAdapter(RedisOverrides, config=ConfigDict(extra="allow", strict=True))
