import asyncio
from collections.abc import Callable
from datetime import timedelta
from typing import Protocol, Self, TypeAlias, cast

from litellm.caching.redis_cache import (
    RedisCache,
    RedisCircuitBreaker,
)
from redis.exceptions import ResponseError

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class _NoopLogger(Protocol):
    async def async_service_success_hook(self, **kwargs: object) -> None: ...

    async def async_service_failure_hook(self, **kwargs: object) -> None: ...

    def service_success_hook(self, **kwargs: object) -> None: ...

    def service_failure_hook(self, **kwargs: object) -> None: ...


class _Logger:
    async def async_service_success_hook(self, **_kwargs: object) -> None:
        return None

    async def async_service_failure_hook(self, **_kwargs: object) -> None:
        return None

    def service_success_hook(self, **_kwargs: object) -> None:
        return None

    def service_failure_hook(self, **_kwargs: object) -> None:
        return None


def _seconds(value: object) -> int:
    if isinstance(value, timedelta):
        return int(value.total_seconds())
    if isinstance(value, bool) or not isinstance(value, int):
        raise TypeError("the TTL must be an int or a timedelta")
    return value


def _set_command(name: str, value: str, *, nx: bool, ex: object) -> list[str]:
    command = ["SET", name, value]
    if nx:
        command.append("NX")
    if ex is not None:
        command.extend(["EX", str(_seconds(ex))])
    return command


class AsyncRecorder:
    def __init__(self) -> None:
        self.commands: list[list[str]] = []

    async def set(
        self, name: str, value: str, *, nx: bool = False, ex: object = None
    ) -> bool:
        self.commands.append(_set_command(name, value, nx=nx, ex=ex))
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
        self.commands.append(["EXPIRE", key, str(_seconds(time))])
        return True


class RecordingPipeline:
    def __init__(self, recorder: AsyncRecorder) -> None:
        self._recorder = recorder

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(self, kind: object, value: object, traceback: object) -> None:
        _ = (kind, value, traceback)

    def set(self, name: str, value: str, ex: object = None) -> None:
        self._recorder.commands.append(_set_command(name, value, nx=False, ex=ex))

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
    namespace: str | None
    default_ttl: int | None
    redis_flush_size: int
    redis_version: str
    redis_batch_writing_buffer: list[tuple[str, Json]]
    _circuit_breaker: RedisCircuitBreaker  # pyright: ignore[reportPrivateUsage]
    service_logger_obj: _NoopLogger
    init_async_client: Callable[[], AsyncRecorder]
    redis_client: SyncRecorder

    async def async_set_cache(
        self, key: str | None, value: Json, **kwargs: object
    ) -> object: ...

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


def _entries(value: Json) -> list[dict[str, Json]]:
    if value is None:
        return []
    if not isinstance(value, list):
        raise ValueError("entries must be a list")
    return [object_value(entry, "entry") for entry in value]


def _replies(value: Json) -> list[str | None]:
    if value is None:
        return []
    if not isinstance(value, list):
        raise ValueError("replies must be a list")
    return [optional_string(reply, "reply") for reply in value]


def _keys(value: Json) -> list[str | None]:
    if value is None:
        return []
    if not isinstance(value, list):
        raise ValueError("keys must be a list")
    return [optional_string(key, "key") for key in value]


def _entry_ttl(entry: dict[str, Json]) -> float | None:
    return optional_number(entry.get("ttl"), "ttl")


def _required_key(entry: dict[str, Json]) -> str:
    return string_value(entry.get("key"), "key")


def _first_entry(entries: list[dict[str, Json]], operation: str) -> dict[str, Json]:
    if not entries:
        raise ValueError(f"{operation} needs an entry")
    return entries[0]


def _members(entry: dict[str, Json]) -> list[str]:
    raw = entry.get("value")
    if not isinstance(raw, list):
        raise ValueError("the members must be a list")
    return [string_value(member, "member") for member in raw]


def _build_cache(input_value: dict[str, Json]) -> tuple[Cache, AsyncRecorder]:
    recorder = AsyncRecorder()
    cache = object.__new__(RedisCache)
    flush_size = optional_integer(input_value.get("flush_size"), "flush_size")
    namespace = optional_string(input_value.get("namespace"), "namespace")
    default_ttl = optional_integer(input_value.get("default_ttl"), "default_ttl")
    object.__setattr__(cache, "namespace", namespace)
    object.__setattr__(cache, "default_ttl", default_ttl)
    object.__setattr__(cache, "redis_flush_size", 100 if flush_size is None else flush_size)
    object.__setattr__(cache, "redis_version", "Unknown")
    object.__setattr__(cache, "redis_batch_writing_buffer", [])
    breaker = RedisCircuitBreaker(failure_threshold=1, recovery_timeout=60)
    object.__setattr__(cache, "_circuit_breaker", breaker)
    object.__setattr__(cache, "service_logger_obj", _Logger())
    object.__setattr__(cache, "init_async_client", lambda: recorder)
    return cast(Cache, cache), recorder


def _build_read_cache(input_value: dict[str, Json], recorder: SyncRecorder) -> Cache:
    cache = object.__new__(RedisCache)
    namespace = optional_string(input_value.get("namespace"), "namespace")
    object.__setattr__(cache, "namespace", namespace)
    object.__setattr__(cache, "redis_version", "Unknown")
    breaker = RedisCircuitBreaker(failure_threshold=1, recovery_timeout=60)
    object.__setattr__(cache, "_circuit_breaker", breaker)
    object.__setattr__(cache, "service_logger_obj", _Logger())
    object.__setattr__(cache, "redis_client", recorder)
    return cast(Cache, cache)


async def _run_write(input_value: dict[str, Json], cache: Cache) -> None:
    operation = string_value(input_value.get("operation"), "operation")
    entries = _entries(input_value.get("entries"))
    match operation:
        case "async_set_cache":
            entry = _first_entry(entries, operation)
            nx = optional_bool(input_value.get("nx"), "nx")
            await cache.async_set_cache(
                optional_string(entry.get("key"), "key"),
                entry.get("value"),
                ttl=_entry_ttl(entry),
                nx=nx,
            )
        case "async_set_cache_pipeline":
            ttl = _entry_ttl(entries[0]) if entries else None
            pairs = [(_required_key(entry), entry.get("value")) for entry in entries]
            await cache.async_set_cache_pipeline(pairs, ttl)
        case "async_set_cache_pipeline_with_ttls":
            triples = [
                (_required_key(entry), entry.get("value"), _entry_ttl(entry))
                for entry in entries
            ]
            await cache.async_set_cache_pipeline_with_ttls(triples)
        case "async_set_cache_sadd":
            entry = _first_entry(entries, operation)
            await cache.async_set_cache_sadd(
                _required_key(entry), _members(entry), _entry_ttl(entry)
            )
        case "batch_cache_write":
            for entry in entries:
                await cache.batch_cache_write(_required_key(entry), entry.get("value"))
        case _:
            raise ValueError(f"unknown operation {operation}")


def _keys_sent(recorder: SyncRecorder, name: str) -> list[str]:
    for command in recorder.commands:
        if command and command[0] == name:
            return command[1:]
    return []


def _write_commands(input_value: dict[str, Json]) -> Json:
    cache, recorder = _build_cache(input_value)
    asyncio.run(_run_write(input_value, cache))
    return {
        "commands": cast(Json, recorder.commands),
        "buffered": len(cache.redis_batch_writing_buffer),
    }


def _read_values(input_value: dict[str, Json]) -> Json:
    operation = string_value(input_value.get("operation"), "operation")
    recorder = SyncRecorder(_replies(input_value.get("replies")))
    cache = _build_read_cache(input_value, recorder)
    match operation:
        case "get_cache":
            keys = _keys(input_value.get("keys"))
            key = keys[0] if keys else None
            if key is None:
                raise ValueError("get_cache needs a key")
            return cast(
                Json,
                {
                    "values": cache.get_cache(key),
                    "keys_sent": _keys_sent(recorder, "GET"),
                },
            )
        case "batch_get_cache":
            values = cache.batch_get_cache(_keys(input_value.get("keys")))
            return cast(
                Json,
                {
                    "values": values,
                    "keys_sent": _keys_sent(recorder, "MGET"),
                },
            )
        case _:
            raise ValueError(f"unknown operation {operation}")


def run(input_value: dict[str, Json]) -> Json:
    function = string_value(input_value.get("function"), "function")
    match function:
        case "write_commands":
            return _write_commands(input_value)
        case "read_values":
            return _read_values(input_value)
        case _:
            raise ValueError(f"unknown function {function}")
