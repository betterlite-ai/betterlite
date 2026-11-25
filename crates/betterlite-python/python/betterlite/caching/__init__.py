from betterlite.caching.base_cache import BaseCache
from betterlite.caching.caching import (
    Cache,
    CacheMode,
    disable_cache,
    enable_cache,
    print_verbose,
    update_cache,
)
from betterlite.caching.disk_cache import DiskCache
from betterlite.caching.dual_cache import DualCache
from betterlite.caching.in_memory_cache import InMemoryCache
from betterlite.caching.redis_cache import RedisCache
from betterlite.types.caching import LiteLLMCacheType

# litellm keeps these on its package; the star import of this package carries them into betterlite.
cache: Cache | None = None
set_verbose: bool = False
default_in_memory_ttl: float | None = None
default_redis_ttl: float | None = None
default_redis_batch_cache_expiry: float | None = None  # ast-grep-ignore: no-long-names

__all__ = [
    "BaseCache",
    "Cache",
    "CacheMode",
    "DiskCache",
    "DualCache",
    "InMemoryCache",
    "LiteLLMCacheType",
    "RedisCache",
    "cache",
    "default_in_memory_ttl",
    "default_redis_batch_cache_expiry",
    "default_redis_ttl",
    "disable_cache",
    "enable_cache",
    "print_verbose",
    "set_verbose",
    "update_cache",
]
