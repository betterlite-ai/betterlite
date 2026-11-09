import asyncio
from datetime import timedelta
from typing import Protocol, Self, TypeAlias, cast

from litellm.caching.redis_cache import RedisCache, RedisCircuitBreaker
from redis.exceptions import ResponseError

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Logger:
    async def async_service_success_hook(self, **_kwargs: object) -> None:
        return None
    async def async_service_failure_hook(self, **_kwargs: object) -> None:
        return None
    def service_success_hook(self, **_kwargs: object) -> None:
        return None
    def service_failure_hook(self, **_kwargs: object) -> None:
        return None


def seconds(value: object) -> int:
    if isinstance(value, timedelta):
        return int(value.total_seconds())
    if isinstance(value, bool) or not isinstance(value, int):
        raise TypeError("the TTL must be an int or a timedelta")
    return value


def set_command(name: str, value: str, *, nx: bool, ex: object) -> list[str]:
    command = ["SET", name, value]
    if nx:
        command.append("NX")
    if ex is not None:
        command.extend(["EX", str(seconds(ex))])
    return command


class AsyncRecorder:
    def __init__(self) -> None:
        self.commands: list[list[str]] = []
    async def set(self, name: str, value: str, *, nx: bool = False, ex: object = None) -> bool:
        self.commands.append(set_command(name, value, nx=nx, ex=ex))
        return True
    def pipeline(self, *, transaction: bool = False) -> "RecordingPipeline":
        _ = transaction
        return RecordingPipeline(self)
    async def sadd(self, key: str, *values: str) -> int:
        self.commands.append(["SADD", key, *values])
        if not values:
            raise ResponseError("wrong number of arguments for 'sadd' command")
        return len(values)
    async def expire(self, key: str, time: object) -> bool:
        self.commands.append(["EXPIRE", key, str(seconds(time))])
        return True


class RecordingPipeline:
    def __init__(self, recorder: AsyncRecorder) -> None:
        self._recorder = recorder
    async def __aenter__(self) -> Self:
        return self
    async def __aexit__(self, kind: object, value: object, traceback: object) -> None:
        _ = (kind, value, traceback)
    def set(self, name: str, value: str, ex: object = None) -> None:
        self._recorder.commands.append(set_command(name, value, nx=False, ex=ex))
    async def execute(self) -> list[bool]:
        return [True]


class SyncRecorder:
    def __init__(self, replies: list[str | None]) -> None:
        self.replies = replies
        self.commands: list[list[str]] = []
    def get(self, name: str) -> bytes | None:
        self.commands.append(["GET", name])
        reply = self.replies[0] if self.replies else None
        return None if reply is None else reply.encode()
    def mget(self, keys: list[str]) -> list[bytes | None]:
        self.commands.append(["MGET", *keys])
        return [None if reply is None else reply.encode() for reply in self.replies]


class Cache(Protocol):
    redis_batch_writing_buffer: list[tuple[str, Json]]
    async def async_set_cache(self, key: str | None, value: Json, **kwargs: object) -> object: ...
    async def async_set_cache_pipeline(
        self, cache_list: list[tuple[str, Json]], ttl: float | None = None
    ) -> None: ...
    async def async_set_cache_pipeline_with_ttls(
        self, cache_list: list[tuple[str, Json, float | None]]
    ) -> None: ...
    async def async_set_cache_sadd(
        self, key: str, value: list[str], ttl: float | None
    ) -> None: ...
    async def batch_cache_write(self, key: str, value: Json) -> None: ...
    def get_cache(self, key: str) -> Json: ...
    def batch_get_cache(self, key_list: list[str | None]) -> dict[str, Json]: ...


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def optional_string(value: Json, name: str) -> str | None:
    if value is None:
        return None
    return string_value(value, name)


def optional_integer(value: Json, name: str) -> int | None:
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{name} must be an integer")
    return value


def optional_number(value: Json, name: str) -> float | None:
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be a number")
    return float(value)


def optional_bool(value: Json, name: str) -> bool:
    if value is None:
        return False
    if not isinstance(value, bool):
        raise ValueError(f"{name} must be a boolean")
    return value


def list_value(value: Json, name: str) -> list[Json]:
    if value is None:
        return []
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def entry_ttl(entry: dict[str, Json]) -> float | None:
    return optional_number(entry.get("ttl"), "ttl")


def required_key(entry: dict[str, Json]) -> str:
    return string_value(entry.get("key"), "key")


def first_entry(entries: list[dict[str, Json]], operation: str) -> dict[str, Json]:
    if not entries:
        raise ValueError(f"{operation} needs an entry")
    return entries[0]


def members(entry: dict[str, Json]) -> list[str]:
    raw = entry.get("value")
    if not isinstance(raw, list):
        raise ValueError("the members must be a list")
    return [string_value(member, "member") for member in raw]


def build_cache(input_value: dict[str, Json], recorder: AsyncRecorder | SyncRecorder) -> Cache:
    cache = object.__new__(RedisCache)
    flush_size = optional_integer(input_value.get("flush_size"), "flush_size")
    breaker = RedisCircuitBreaker(failure_threshold=1, recovery_timeout=60)
    fields: dict[str, object] = {
        "namespace": optional_string(input_value.get("namespace"), "namespace"),
        "default_ttl": optional_integer(input_value.get("default_ttl"), "default_ttl"),
        "redis_flush_size": 100 if flush_size is None else flush_size,
        "redis_version": "Unknown",
        "redis_batch_writing_buffer": [],
        "_circuit_breaker": breaker,
        "service_logger_obj": Logger(),
        "init_async_client": lambda: recorder,
        "redis_client": recorder,
    }
    for name, value in fields.items():
        object.__setattr__(cache, name, value)
    return cast(Cache, cache)


async def run_write(input_value: dict[str, Json], cache: Cache) -> None:
    operation = string_value(input_value.get("operation"), "operation")
    raw_entries = list_value(input_value.get("entries"), "entries")
    entries = [object_value(entry, "entry") for entry in raw_entries]
    match operation:
        case "async_set_cache":
            entry = first_entry(entries, operation)
            await cache.async_set_cache(
                optional_string(entry.get("key"), "key"),
                entry.get("value"),
                ttl=entry_ttl(entry),
                nx=optional_bool(input_value.get("nx"), "nx"),
            )
        case "async_set_cache_pipeline":
            ttl = entry_ttl(entries[0]) if entries else None
            pairs = [(required_key(entry), entry.get("value")) for entry in entries]
            await cache.async_set_cache_pipeline(pairs, ttl)
        case "async_set_cache_pipeline_with_ttls":
            triples = [
                (required_key(entry), entry.get("value"), entry_ttl(entry)) for entry in entries
            ]
            await cache.async_set_cache_pipeline_with_ttls(triples)
        case "async_set_cache_sadd":
            entry = first_entry(entries, operation)
            await cache.async_set_cache_sadd(required_key(entry), members(entry), entry_ttl(entry))
        case "batch_cache_write":
            for entry in entries:
                await cache.batch_cache_write(required_key(entry), entry.get("value"))
        case _:
            raise ValueError(f"unknown operation {operation}")


def keys_sent(recorder: SyncRecorder, name: str) -> list[str]:
    found = next((command for command in recorder.commands if command and command[0] == name), None)
    return [] if found is None else found[1:]


def write_commands(input_value: dict[str, Json]) -> Json:
    recorder = AsyncRecorder()
    cache = build_cache(input_value, recorder)
    asyncio.run(run_write(input_value, cache))
    return {
        "commands": cast(Json, recorder.commands),
        "buffered": len(cache.redis_batch_writing_buffer),
    }


def read_values(input_value: dict[str, Json]) -> Json:
    operation = string_value(input_value.get("operation"), "operation")
    raw_replies = list_value(input_value.get("replies"), "replies")
    replies = [optional_string(reply, "reply") for reply in raw_replies]
    recorder = SyncRecorder(replies)
    cache = build_cache(input_value, recorder)
    keys = [optional_string(key, "key") for key in list_value(input_value.get("keys"), "keys")]
    match operation:
        case "get_cache":
            key = keys[0] if keys else None
            if key is None:
                raise ValueError("get_cache needs a key")
            return cast(
                Json, {"values": cache.get_cache(key), "keys_sent": keys_sent(recorder, "GET")}
            )
        case "batch_get_cache":
            values = cache.batch_get_cache(keys)
            return cast(Json, {"values": values, "keys_sent": keys_sent(recorder, "MGET")})
        case _:
            raise ValueError(f"unknown operation {operation}")


def run(input_value: dict[str, Json]) -> Json:
    function = string_value(input_value.get("function"), "function")
    match function:
        case "write_commands":
            return write_commands(input_value)
        case "read_values":
            return read_values(input_value)
        case _:
            raise ValueError(f"unknown function {function}")
