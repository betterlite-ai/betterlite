from typing import Literal, TypedDict

import litellm
from litellm.caching.redis_cache import RedisCache

# RedisCache's client is lazy and its failed info() and ping() calls are caught.
CLOSED_PORT = 6399
CONNECT_TIMEOUT_SECONDS = 0.1


class Input(TypedDict):
    function: Literal["cache_defaults"]
    default_redis_ttl: float | None
    redis_flush_size: int | None


def run(input_value: Input) -> dict[str, int]:
    previous_ttl = litellm.default_redis_ttl
    litellm.default_redis_ttl = input_value["default_redis_ttl"]
    try:
        cache = RedisCache(
            host="127.0.0.1",
            port=CLOSED_PORT,
            socket_timeout=CONNECT_TIMEOUT_SECONDS,
            redis_flush_size=input_value["redis_flush_size"],
        )
    finally:
        litellm.default_redis_ttl = previous_ttl
    return {"default_ttl": cache.default_ttl, "redis_flush_size": cache.redis_flush_size}
