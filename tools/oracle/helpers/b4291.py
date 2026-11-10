import asyncio
import importlib.util
import sys
import time
from collections.abc import Generator
from contextlib import contextmanager
from tempfile import TemporaryDirectory
from types import ModuleType
from typing import TypeAlias, cast

from litellm.caching.disk_cache import DiskCache
from litellm.proxy.common_utils.cache_coordinator import (
    AsyncCacheProtocol,
    EventDrivenCacheCoordinator,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Value: TypeAlias = bool | int | float | str | list["Value"] | dict[str, "Value"]


class StorageStandIn:
    # diskcache 5.6.3 stores expire_time = now + expire and reads a row only while expire_time > now
    def __init__(self, directory: str) -> None:
        self.directory = directory
        self.rows: dict[str, tuple[Json, float | None]] = {}

    def set(self, key: str, value: Json, expire: float | None = None) -> bool:
        expire_time = None if expire is None else time.time() + expire
        self.rows[key] = (value, expire_time)
        return True

    def get(self, key: str, default: Json = None) -> Json:
        return self._live(key, default)

    def pop(self, key: str, default: Json = None) -> Json:
        value = self._live(key, default)
        self.rows.pop(key, None)
        return value

    def clear(self) -> int:
        count = len(self.rows)
        self.rows.clear()
        return count

    @contextmanager
    def transact(self) -> Generator[None, None, None]:
        yield

    def _live(self, key: str, default: Json) -> Json:
        stored = self.rows.get(key)
        if stored is None:
            return default
        (value, expire_time) = stored
        if expire_time is not None and expire_time <= time.time():
            return default
        return value


def disk_storage() -> None:
    # the oracle pins litellm[proxy], which leaves out the caching extra that carries diskcache
    if "diskcache" in sys.modules or importlib.util.find_spec("diskcache") is not None:
        return
    module = ModuleType("diskcache")
    setattr(module, "Cache", StorageStandIn)  # noqa: B010 - pyright rejects unknown ModuleType attributes
    sys.modules["diskcache"] = module


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def list_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def value_of(value: Json, name: str) -> Value:
    if value is None:
        raise ValueError(f"{name} must not be null")
    return cast(Value, value)


def number_value(value: Json, name: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be a number")
    return float(value)


def integer_value(value: Json, name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{name} must be an integer")
    return value


def disk_cache(directory: str) -> DiskCache:
    disk_storage()
    return DiskCache(disk_cache_dir=directory)


def _set_and_get(data: dict[str, Json]) -> Json:
    key = string_value(data.get("key"), "key")
    value = data.get("value")
    with TemporaryDirectory() as directory:
        cache = disk_cache(directory)
        cache.set_cache(key, value)  # pyright: ignore[reportUnknownMemberType]
        read = cast(Json, cache.get_cache(key))  # pyright: ignore[reportUnknownMemberType]
        missing = cast(Json, cache.get_cache("absent-key"))  # pyright: ignore[reportUnknownMemberType]
    return cast(Json, {"read": read, "missing": missing})


def _ttl_expiry(data: dict[str, Json]) -> Json:
    entries = list_value(data.get("entries"), "entries")
    reads: list[Json] = []
    with TemporaryDirectory() as directory:
        cache = disk_cache(directory)
        for entry in entries:
            record = object_value(entry, "entry")
            key = string_value(record.get("key"), "key")
            ttl = number_value(record.get("ttl"), "ttl")
            cache.set_cache(key, record.get("value"), ttl=ttl)  # pyright: ignore[reportUnknownMemberType]
            reads.append(cast(Json, cache.get_cache(key)))  # pyright: ignore[reportUnknownMemberType]
    return cast(Json, {"reads": reads})


def _truthy_reads(data: dict[str, Json]) -> Json:
    values = list_value(data.get("values"), "values")
    reads: list[Json] = []
    with TemporaryDirectory() as directory:
        cache = disk_cache(directory)
        for index, value in enumerate(values):
            key = f"key-{index}"
            cache.set_cache(key, value)  # pyright: ignore[reportUnknownMemberType]
            reads.append(cast(Json, cache.get_cache(key)))  # pyright: ignore[reportUnknownMemberType]
    return cast(Json, {"reads": reads})


def _batch_get(data: dict[str, Json]) -> Json:
    entries = list_value(data.get("entries"), "entries")
    keys = [string_value(key, "key") for key in list_value(data.get("keys"), "keys")]
    with TemporaryDirectory() as directory:
        cache = disk_cache(directory)
        for entry in entries:
            record = object_value(entry, "entry")
            cache.set_cache(  # pyright: ignore[reportUnknownMemberType]
                string_value(record.get("key"), "key"), record.get("value")
            )
        reads = cast(Json, cache.batch_get_cache(keys))  # pyright: ignore[reportUnknownMemberType]
    return cast(Json, {"reads": reads})


def _increment(data: dict[str, Json]) -> Json:
    key = string_value(data.get("key"), "key")
    raw = list_value(data.get("increments"), "increments")
    increments = [integer_value(entry, "increments") for entry in raw]
    results: list[Json] = []
    with TemporaryDirectory() as directory:
        cache = disk_cache(directory)
        if "initial" in data:
            cache.set_cache(key, data["initial"])  # pyright: ignore[reportUnknownMemberType]
        for step in increments:
            # every increment must run in order against the same key
            results.append(cast(Json, cache.increment_cache(key, step)))  # noqa: PERF401  # pyright: ignore[reportUnknownMemberType]
        stored = cast(Json, cache.get_cache(key))  # pyright: ignore[reportUnknownMemberType]
    return cast(Json, {"results": results, "stored": stored})


def _delete(data: dict[str, Json]) -> Json:
    key = string_value(data.get("key"), "key")
    kept = object_value(data.get("kept"), "kept")
    kept_key = string_value(kept.get("key"), "key")
    with TemporaryDirectory() as directory:
        cache = disk_cache(directory)
        cache.set_cache(key, data.get("value"))  # pyright: ignore[reportUnknownMemberType]
        cache.set_cache(kept_key, kept.get("value"))  # pyright: ignore[reportUnknownMemberType]
        cache.delete_cache(key)  # pyright: ignore[reportUnknownMemberType]
        cache.delete_cache("absent-key")  # pyright: ignore[reportUnknownMemberType]
        deleted = cast(Json, cache.get_cache(key))  # pyright: ignore[reportUnknownMemberType]
        still_kept = cast(Json, cache.get_cache(kept_key))  # pyright: ignore[reportUnknownMemberType]
    return cast(Json, {"deleted": deleted, "kept": still_kept})


class _DictCache:
    def __init__(self) -> None:
        self.values: dict[str, Value] = {}

    async def async_get_cache(
        self,
        key: str,
        parent_otel_span: object = None,  # noqa: ARG002 - the protocol names this parameter
        local_only: bool = False,  # noqa: ARG002, FBT001, FBT002 - the protocol names this parameter
        **kwargs: object,  # noqa: ARG002 - the coordinator sends no extra options
    ) -> Value | None:
        return self.values.get(key)

    async def async_set_cache(
        self,
        key: str,
        value: Value,
        local_only: bool = False,  # noqa: ARG002, FBT001, FBT002 - the protocol names this parameter
        **kwargs: object,  # noqa: ARG002 - the coordinator sends no extra options
    ) -> object:
        self.values[key] = value
        return None


class _BlockingLoader:
    def __init__(self, value: Value, loads: list[Value]) -> None:
        self.value = value
        self.loads = loads
        self.started = asyncio.Event()
        self.release = asyncio.Event()

    async def load(self) -> Value:
        self.loads.append(self.value)
        self.started.set()
        await self.release.wait()
        return self.value


class _UnblockedLoader:
    def __init__(self, value: Value, loads: list[Value]) -> None:
        self.value = value
        self.loads = loads

    async def load(self) -> Value:
        self.loads.append(self.value)
        return self.value


class _FailingLoader:
    def __init__(self, loads: list[Value]) -> None:
        self.loads = loads
        self.started = asyncio.Event()
        self.release = asyncio.Event()

    async def load(self) -> Value:
        self.loads.append("key")
        self.started.set()
        await self.release.wait()
        raise ValueError("load failed")


async def _settle() -> None:
    for _ in range(8):
        await asyncio.sleep(0)


async def _single_loader(value: Value, requests: int) -> Json:
    cache: AsyncCacheProtocol[Value] = _DictCache()
    coordinator = EventDrivenCacheCoordinator(log_prefix="[CACHE]")
    loads: list[Value] = []
    loader = _BlockingLoader(value, loads)
    first = asyncio.create_task(coordinator.get_or_load("key", cache, loader.load))
    await loader.started.wait()
    waiters = [
        asyncio.create_task(
            coordinator.get_or_load("key", cache, _UnblockedLoader(value, loads).load)
        )
        for _ in range(max(requests - 1, 0))
    ]
    await _settle()
    loader.release.set()
    first_value = await first
    waiter_values = list(await asyncio.gather(*waiters))
    return cast(Json, {"loads": len(loads), "results": [first_value, *waiter_values]})


async def _waiter_gets_cached(value: Value) -> Json:
    cache: AsyncCacheProtocol[Value] = _DictCache()
    coordinator = EventDrivenCacheCoordinator(log_prefix="[CACHE]")
    loads: list[Value] = []
    loader = _BlockingLoader(value, loads)
    first = asyncio.create_task(coordinator.get_or_load("key", cache, loader.load))
    await loader.started.wait()
    waiter = asyncio.create_task(
        coordinator.get_or_load("key", cache, _UnblockedLoader(value, loads).load)
    )
    await _settle()
    loader.release.set()
    results = [await first, await waiter]
    cached = await cache.async_get_cache("key")
    return cast(Json, {"loads": len(loads), "results": results, "cached": cached})


async def _loader_failure(value: Value) -> Json:
    cache: AsyncCacheProtocol[Value] = _DictCache()
    coordinator = EventDrivenCacheCoordinator(log_prefix="[CACHE]")
    loads: list[Value] = []
    loader = _FailingLoader(loads)
    first = asyncio.create_task(coordinator.get_or_load("key", cache, loader.load))
    await loader.started.wait()
    waiter = asyncio.create_task(
        coordinator.get_or_load("key", cache, _UnblockedLoader(value, loads).load)
    )
    await _settle()
    loader.release.set()
    failed = False
    try:
        await first
    except ValueError:
        failed = True
    waited = await waiter
    retried = await coordinator.get_or_load(
        "key", cache, _UnblockedLoader(value, loads).load
    )
    return cast(
        Json,
        {"failed": failed, "waiter": waited, "retry": retried, "loads": len(loads)},
    )


def run(  # noqa: C901, PLR0911 - one branch per b4291 operation
    input_value: dict[str, Json],
) -> Json:
    (function, payload) = next(iter(input_value.items()))
    if len(input_value) != 1:
        raise ValueError("a b4291 call names exactly one operation")
    data = {} if payload is None else object_value(payload, function)
    match function:
        case "disk_set_and_get":
            return _set_and_get(data)
        case "disk_ttl":
            return _ttl_expiry(data)
        case "disk_truthy_reads":
            return _truthy_reads(data)
        case "disk_batch_get":
            return _batch_get(data)
        case "disk_increment":
            return _increment(data)
        case "disk_delete":
            return _delete(data)
        case "coordinator_single_loader":
            requests = integer_value(data.get("requests"), "requests")
            return asyncio.run(_single_loader(value_of(data.get("value"), "value"), requests))
        case "coordinator_waiter_gets_cached":
            return asyncio.run(_waiter_gets_cached(value_of(data.get("value"), "value")))
        case "coordinator_loader_failure":
            return asyncio.run(_loader_failure(value_of(data.get("value"), "value")))
        case _:
            raise ValueError(f"unknown function {function}")
