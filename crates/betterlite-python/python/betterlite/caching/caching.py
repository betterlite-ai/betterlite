import logging
from contextlib import suppress
from enum import Enum
from typing import Final

from pydantic import JsonValue
from typing_extensions import Unpack

import betterlite
from betterlite._cache_ops import CacheOperations
from betterlite._settings.callbacks import Callback
from betterlite.caching._backend import cluster_nodes_are_set
from betterlite.caching._cache_arguments import REDIS_OVERRIDES, CacheArguments
from betterlite.caching.base_cache import BaseCache
from betterlite.caching.disk_cache import DiskCache
from betterlite.caching.in_memory_cache import InMemoryCache
from betterlite.caching.redis_cache import RedisCache
from betterlite.types.caching import DEFAULT_CACHING_SUPPORTED_CALL_TYPES as SUPPORTED_CALLS
from betterlite.types.caching import CachingSupportedCallTypes as CallType
from betterlite.types.caching import LiteLLMCacheType, SemanticCacheScope

_LOGGER: Final = logging.getLogger("LiteLLM")
_CACHE_CALLBACK: Final = "cache"


def print_verbose(print_statement: JsonValue) -> None:
    # LiteLLM swallows every failure in this line so a broken log handler can't fail a completion.
    with suppress(Exception):
        _LOGGER.debug("%s", print_statement)
        if betterlite.set_verbose:
            print(print_statement)  # noqa: T201  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/caching.py#L62-L68


class CacheMode(str, Enum):
    default_on = "default_on"
    default_off = "default_off"


# LiteLLM's own signatures put the cache callbacks on these three shared lists.
def _callback_lists() -> tuple[list[Callback], list[Callback], list[Callback]]:
    return (
        betterlite.input_callback,
        betterlite.success_callback,
        betterlite._async_success_callback,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/caching.py#L1044-L1047
    )


def _add_cache_callbacks() -> None:
    for entries in _callback_lists():
        if _CACHE_CALLBACK not in entries:
            entries.append(_CACHE_CALLBACK)


class Cache(CacheOperations):
    def __init__(  # noqa: PLR0913, PLR0917, C901  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/caching.py#L80-L132
        self,
        type: LiteLLMCacheType | str | None = LiteLLMCacheType.LOCAL,  # noqa: A002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/caching.py#L82
        mode: CacheMode | str | None = CacheMode.default_on,
        host: str | None = None,
        port: str | None = None,
        password: str | None = None,
        namespace: str | None = None,
        ttl: float | None = None,
        default_in_memory_ttl: float | None = None,
        default_in_redis_ttl: float | None = None,
        similarity_threshold: float | None = None,
        supported_call_types: list[CallType] | None = list(SUPPORTED_CALLS),  # noqa: B006  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/caching.py#L93
        azure_account_url: str | None = None,
        azure_blob_container: str | None = None,
        s3_bucket_name: str | None = None,
        s3_region_name: str | None = None,
        s3_api_version: str | None = None,
        s3_use_ssl: bool | None = True,  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/caching.py#L100
        s3_verify: bool | str | None = None,  # noqa: FBT001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/caching.py#L101
        s3_endpoint_url: str | None = None,
        s3_aws_access_key_id: str | None = None,
        s3_aws_secret_access_key: str | None = None,
        s3_aws_session_token: str | None = None,
        # ast-grep-ignore: no-loose-types - LiteLLM declares this parameter as object
        s3_config: object | None = None,
        s3_path: str | None = None,
        gcs_bucket_name: str | None = None,
        gcs_path_service_account: str | None = None,
        gcs_path: str | None = None,
        # ast-grep-ignore: no-long-names
        redis_semantic_cache_embedding_model: str = "text-embedding-ada-002",
        # ast-grep-ignore: no-long-names
        redis_semantic_cache_index_name: str | None = None,
        # ast-grep-ignore: no-long-names
        valkey_semantic_cache_embedding_model: str = "text-embedding-ada-002",
        # ast-grep-ignore: no-long-names
        valkey_semantic_cache_index_name: str | None = None,
        redis_flush_size: int | None = None,
        redis_startup_nodes: list[JsonValue] | None = None,
        disk_cache_dir: str | None = None,
        qdrant_api_base: str | None = None,
        qdrant_api_key: str | None = None,
        qdrant_collection_name: str | None = None,
        qdrant_quantization_config: str | None = None,
        # ast-grep-ignore: no-long-names
        qdrant_semantic_cache_embedding_model: str = "text-embedding-ada-002",
        # ast-grep-ignore: no-long-names
        qdrant_semantic_cache_vector_size: int | None = None,
        # ast-grep-ignore: no-long-names
        semantic_cache_embedding_max_input_tokens: int | None = None,
        # ast-grep-ignore: no-long-names
        semantic_cache_embedding_timeout: float | None = None,
        semantic_cache_scope: str = SemanticCacheScope.KEY.value,
        gcp_service_account: str | None = None,
        gcp_ssl_ca_certs: str | None = None,
        _backend: BaseCache | None = None,
        **kwargs: JsonValue,
    ) -> None:
        # The semantic, S3, GCS and Azure backends are later tasks; their arguments stay unused.
        _unread = (
            similarity_threshold,
            azure_account_url,
            azure_blob_container,
            s3_bucket_name,
            s3_region_name,
            s3_api_version,
            s3_use_ssl,
            s3_verify,
            s3_endpoint_url,
            s3_aws_access_key_id,
            s3_aws_secret_access_key,
            s3_aws_session_token,
            s3_config,
            s3_path,
            gcs_bucket_name,
            gcs_path_service_account,
            gcs_path,
            redis_semantic_cache_embedding_model,  # ast-grep-ignore: no-long-names
            redis_semantic_cache_index_name,  # ast-grep-ignore: no-long-names
            valkey_semantic_cache_embedding_model,  # ast-grep-ignore: no-long-names
            valkey_semantic_cache_index_name,  # ast-grep-ignore: no-long-names
            qdrant_api_base,
            qdrant_api_key,
            qdrant_collection_name,
            qdrant_quantization_config,
            qdrant_semantic_cache_embedding_model,  # ast-grep-ignore: no-long-names
            qdrant_semantic_cache_vector_size,  # ast-grep-ignore: no-long-names
            semantic_cache_embedding_max_input_tokens,  # ast-grep-ignore: no-long-names
            semantic_cache_embedding_timeout,  # ast-grep-ignore: no-long-names
            gcp_service_account,
            gcp_ssl_ca_certs,
        )
        selected = _selected_type(type)
        selected_text = selected.value
        backend: BaseCache
        if _backend is not None:
            backend = _backend
        elif selected == LiteLLMCacheType.REDIS:
            if redis_startup_nodes or cluster_nodes_are_set():
                raise NotImplementedError("the redis cluster cache is not built yet")
            overrides = REDIS_OVERRIDES.validate_python(kwargs)
            client_options = {
                key: value for key, value in kwargs.items()
                if key not in ("startup_nodes", "socket_timeout")
            }
            backend = RedisCache(
                host=host,
                port=port,
                password=password,
                redis_flush_size=redis_flush_size,
                namespace=namespace,
                startup_nodes=overrides.get("startup_nodes"),
                socket_timeout=overrides["socket_timeout"],
                **client_options,
            )
        elif selected == LiteLLMCacheType.LOCAL:
            backend = InMemoryCache()
        elif selected == LiteLLMCacheType.DISK:
            backend = DiskCache(disk_cache_dir=disk_cache_dir)
        else:
            raise NotImplementedError(f"{selected_text} cache type is not built yet")
        self.cache = backend
        _add_cache_callbacks()
        self.supported_call_types = supported_call_types
        self.type = selected
        self.namespace = namespace
        self.redis_flush_size = redis_flush_size
        self.ttl = ttl
        self.mode = CacheMode.default_on if mode is None else mode
        self.semantic_cache_scope = SemanticCacheScope(semantic_cache_scope).value
        if self.type == LiteLLMCacheType.LOCAL and default_in_memory_ttl is not None:
            self.ttl = default_in_memory_ttl
        if self.type in (
            LiteLLMCacheType.REDIS,
            LiteLLMCacheType.REDIS_SEMANTIC,
            LiteLLMCacheType.VALKEY_SEMANTIC,
        ) and default_in_redis_ttl is not None:
            self.ttl = default_in_redis_ttl
        if self.namespace is not None:
            match backend:
                case RedisCache():
                    backend.namespace = self.namespace
                case BaseCache():
                    ...


def _selected_type(cache_type: LiteLLMCacheType | str | None) -> LiteLLMCacheType:
    if cache_type is None:
        return LiteLLMCacheType.LOCAL
    try:
        return LiteLLMCacheType(cache_type)
    except ValueError:
        match cache_type:
            case LiteLLMCacheType():
                message = f"{cache_type.value} cache type is not built yet"
                raise NotImplementedError(message) from None
            case str():
                message = f"{cache_type} cache type is not built yet"
                raise NotImplementedError(message) from None


def enable_cache(
    type: LiteLLMCacheType | str | None = LiteLLMCacheType.LOCAL,  # noqa: A002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/caching.py#L1016
    host: str | None = None,
    port: str | None = None,
    password: str | None = None,
    supported_call_types: list[CallType] | None = list(SUPPORTED_CALLS),  # noqa: B006  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/caching.py#L1020
    **kwargs: Unpack[CacheArguments],
) -> None:
    print_verbose("LiteLLM: Enabling Cache")
    _add_cache_callbacks()
    if betterlite.cache is None:
        betterlite.cache = Cache(
            type=type,
            host=host,
            port=port,
            password=password,
            supported_call_types=supported_call_types,
            **kwargs,
        )
    print_verbose(f"LiteLLM: Cache enabled, litellm.cache={betterlite.cache}")
    print_verbose(f"LiteLLM Cache: {betterlite.cache}")


def update_cache(
    type: LiteLLMCacheType | str | None = LiteLLMCacheType.LOCAL,  # noqa: A002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/caching.py#L1063
    host: str | None = None,
    port: str | None = None,
    password: str | None = None,
    supported_call_types: list[CallType] | None = list(SUPPORTED_CALLS),  # noqa: B006  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/caching.py#L1067
    **kwargs: Unpack[CacheArguments],
) -> None:
    print_verbose("LiteLLM: Updating Cache")
    betterlite.cache = Cache(
        type=type,
        host=host,
        port=port,
        password=password,
        supported_call_types=supported_call_types,
        **kwargs,
    )
    print_verbose(f"LiteLLM: Cache Updated, litellm.cache={betterlite.cache}")
    print_verbose(f"LiteLLM Cache: {betterlite.cache}")


def disable_cache() -> None:
    print_verbose("LiteLLM: Disabling Cache")
    with suppress(ValueError):
        for entries in _callback_lists():
            entries.remove(_CACHE_CALLBACK)
    betterlite.cache = None
    print_verbose(f"LiteLLM: Cache disabled, litellm.cache={betterlite.cache}")
