import asyncio
from typing import Protocol, TypeAlias, cast

from litellm.caching.redis_cache import RedisCache, RedisCircuitBreaker

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class BatchCache(Protocol):
    async def async_batch_get_cache(self, key_list: list[str | None]) -> dict[str, Json]: ...


class TtlCache(Protocol):
    async def async_get_ttl(self, key: str) -> int | None: ...


class Logger:
    async def async_service_success_hook(self, **_kwargs: object) -> None:
        return None

    async def async_service_failure_hook(self, **_kwargs: object) -> None:
        return None

    def service_success_hook(self, **_kwargs: object) -> None:
        return None

    def service_failure_hook(self, **_kwargs: object) -> None:
        return None


class MgetRecorder:
    def __init__(self, replies: list[str | None]) -> None:
        self.replies = replies
        self.keys: list[str] = []

    async def mget(self, keys: list[str]) -> list[bytes | None]:
        self.keys.extend(keys)
        return [None if reply is None else reply.encode() for reply in self.replies]


class TtlRecorder:
    def __init__(self, reply: int) -> None:
        self.reply = reply
        self.keys: list[str] = []

    async def ttl(self, key: str) -> int:
        self.keys.append(key)
        return self.reply


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def optional_string(value: Json, name: str) -> str | None:
    if value is None:
        return None
    return string_value(value, name)


def integer_value(value: Json, name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{name} must be an integer")
    return value


def list_value(value: Json, name: str) -> list[Json]:
    if value is None:
        return []
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def empty_cache(namespace: str | None) -> RedisCache:
    cache = object.__new__(RedisCache)
    breaker = RedisCircuitBreaker(failure_threshold=1, recovery_timeout=60)
    object.__setattr__(cache, "namespace", namespace)
    object.__setattr__(cache, "_circuit_breaker", breaker)
    object.__setattr__(cache, "service_logger_obj", Logger())
    return cache


def batch_read(input_value: dict[str, Json]) -> Json:
    namespace = optional_string(input_value.get("namespace"), "namespace")
    keys = [optional_string(key, "key") for key in list_value(input_value.get("keys"), "keys")]
    raw_replies = list_value(input_value.get("replies"), "replies")
    replies = [optional_string(reply, "reply") for reply in raw_replies]
    recorder = MgetRecorder(replies)
    cache = cast(BatchCache, empty_cache(namespace))
    object.__setattr__(cache, "_async_run_redis_mget_operation", recorder.mget)
    values: Json = asyncio.run(cache.async_batch_get_cache(keys))
    return {"keys_sent": cast(Json, recorder.keys), "values": values}


def ttl_reply(input_value: dict[str, Json]) -> Json:
    namespace = optional_string(input_value.get("namespace"), "namespace")
    key = string_value(input_value.get("key"), "key")
    reply = integer_value(input_value.get("reply"), "reply")
    recorder = TtlRecorder(reply)
    cache = cast(TtlCache, empty_cache(namespace))
    object.__setattr__(cache, "_async_commands", lambda: recorder)
    ttl: Json = asyncio.run(cache.async_get_ttl(key))
    return {"key_sent": recorder.keys[0], "ttl": ttl}


def run(input_value: dict[str, Json]) -> Json:
    function = string_value(input_value.get("function"), "function")
    match function:
        case "batch_read":
            return batch_read(input_value)
        case "ttl_reply":
            return ttl_reply(input_value)
        case _:
            raise ValueError(f"unknown function {function}")
