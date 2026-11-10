import asyncio
from dataclasses import dataclass
from typing import Self, TypeAlias, cast

from litellm.caching.redis_cache import RedisCache, RedisCircuitBreaker
from litellm.types.caching import RedisPipelineLpopOperation, RedisPipelineRpushOperation
from redis.exceptions import ResponseError

# pyright: reportUnknownMemberType=false

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Value: TypeAlias = str | bytes | int | float


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def optional_integer(value: Json, name: str) -> int | None:
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{name} must be an integer")
    return value


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def list_value(value: Json, name: str) -> list[Json]:
    if value is None:
        return []
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def decode_value(value: Json) -> Value:
    if isinstance(value, dict):
        raw = value.get("bytes")
        if not isinstance(raw, list) or not all(isinstance(item, int) for item in raw):
            raise ValueError("a bytes value is an object with a bytes list")
        return bytes(cast(list[int], raw))
    if isinstance(value, bool) or not isinstance(value, (str, int, float)):
        raise ValueError("a list value must be a string, a number or {bytes: [...]}")
    return value


def as_bytes(value: Value) -> bytes:
    if isinstance(value, bytes):
        return value
    if isinstance(value, str):
        return value.encode()
    return str(value).encode()


def push_rows(lists: dict[str, list[bytes]], name: str, values: list[Value]) -> int:
    rows = lists.setdefault(name, [])
    rows.extend(as_bytes(value) for value in values)
    return len(rows)


def trim_rows(lists: dict[str, list[bytes]], name: str, start: int, stop: int) -> bool:
    rows = lists.get(name)
    if rows is not None:
        length = len(rows)
        begin = max(length + start, 0) if start < 0 else start
        end = min(length + stop if stop < 0 else stop, length - 1)
        if begin > end:
            rows.clear()
        else:
            rows[:] = rows[begin : end + 1]
    return True


def pop_rows(
    lists: dict[str, list[bytes]], name: str, count: int | None
) -> bytes | list[bytes] | None:
    rows = lists.get(name)
    if not rows:
        return None
    if count is None:
        return rows.pop(0)
    if count < 0:
        raise ResponseError("ERR value is out of range, must be positive")
    popped = rows[:count]
    del rows[: len(popped)]
    return popped


@dataclass
class Push:
    name: str
    values: list[Value]


@dataclass
class Trim:
    name: str
    start: int
    stop: int


@dataclass
class Pop:
    name: str
    count: int | None


class FakePipeline:
    def __init__(self, lists: dict[str, list[bytes]], lpop_args: list[int]) -> None:
        self.lists = lists
        self.lpop_args = lpop_args
        self.queued: list[Push | Trim | Pop] = []

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(self, kind: object, value: object, traceback: object) -> None:
        _ = (kind, value, traceback)

    def rpush(self, name: str, *values: Value) -> None:
        self.queued.append(Push(name, list(values)))

    def ltrim(self, name: str, start: int, stop: int) -> None:
        self.queued.append(Trim(name, start, stop))

    def lpop(self, name: str, count: int | None = None) -> None:
        self.lpop_args.append(3 if count is not None else 2)
        self.queued.append(Pop(name, count))

    async def execute(self) -> list[object]:
        results: list[object] = []
        for command in self.queued:
            match command:
                case Push(name=name, values=values):
                    results.append(push_rows(self.lists, name, values))
                case Trim(name=name, start=start, stop=stop):
                    results.append(trim_rows(self.lists, name, start, stop))
                case Pop(name=name, count=count):
                    results.append(pop_rows(self.lists, name, count))
        self.queued.clear()
        return results


class FakeRedis:
    def __init__(self, lists: dict[str, list[bytes]], lpop_args: list[int]) -> None:
        self.lists = lists
        self.lpop_args = lpop_args

    async def rpush(self, name: str, *values: Value) -> int:
        return push_rows(self.lists, name, list(values))

    async def lpop(self, name: str, count: int | None = None) -> bytes | list[bytes] | None:
        self.lpop_args.append(3 if count is not None else 2)
        return pop_rows(self.lists, name, count)

    def pipeline(self, *, transaction: bool = True) -> FakePipeline:
        _ = transaction
        return FakePipeline(self.lists, self.lpop_args)


class RecordingLogger:
    def __init__(self) -> None:
        self.events: list[dict[str, Json]] = []

    async def async_service_success_hook(self, *, call_type: str, **_: object) -> None:
        self.events.append({"call_type": call_type, "outcome": "success"})

    async def async_service_failure_hook(self, *, call_type: str, **_: object) -> None:
        self.events.append({"call_type": call_type, "outcome": "failure"})


def entry_values(entry: dict[str, Json]) -> list[Value]:
    return [decode_value(item) for item in list_value(entry.get("values"), "values")]


def pipeline_entries(input_value: dict[str, Json]) -> list[dict[str, Json]]:
    raw = list_value(input_value.get("operations"), "operations")
    return [object_value(entry, "operation") for entry in raw]


def project_value(value: Json) -> Json:
    if value is None:
        return None
    if isinstance(value, bytes):
        return {"bytes": list(value)}
    if isinstance(value, list):
        return [project_value(item) for item in value]
    return value


async def apply_operation(input_value: dict[str, Json], cache: RedisCache) -> Json:
    operation = string_value(input_value.get("operation"), "operation")
    match operation:
        case "rpush":
            key = string_value(input_value.get("key"), "key")
            return await cache.async_rpush(key, entry_values(input_value))
        case "rpush_and_trim":
            key = string_value(input_value.get("key"), "key")
            max_len = optional_integer(input_value.get("max_len"), "max_len")
            if max_len is None:
                raise ValueError("rpush_and_trim needs max_len")
            return await cache.async_rpush_and_trim(key, entry_values(input_value), max_len)
        case "rpush_pipeline":
            entries = [
                RedisPipelineRpushOperation(
                    key=string_value(entry.get("key"), "key"), values=entry_values(entry)
                )
                for entry in pipeline_entries(input_value)
            ]
            return cast(Json, await cache.async_rpush_pipeline(entries))
        case "lpop":
            key = string_value(input_value.get("key"), "key")
            count = optional_integer(input_value.get("count"), "count")
            return project_value(cast(Json, await cache.async_lpop(key, count)))
        case "lpop_pipeline":
            entries = [
                RedisPipelineLpopOperation(
                    key=string_value(entry.get("key"), "key"),
                    count=optional_integer(entry.get("count"), "count"),
                )
                for entry in pipeline_entries(input_value)
            ]
            result = await cache.async_lpop_pipeline(entries)
            return [project_value(cast(Json, entry)) for entry in result]
        case _:
            raise ValueError(f"unknown operation {operation}")


async def drive(input_value: dict[str, Json], cache: RedisCache) -> Json:
    result = await apply_operation(input_value, cache)
    await asyncio.sleep(0)
    return result


def project_lists(lists: dict[str, list[bytes]]) -> Json:
    return {key: [project_value(cast(Json, item)) for item in rows] for key, rows in lists.items()}


def run(input_value: dict[str, Json]) -> Json:
    raw = input_value.get("lists")
    lists = (
        {}
        if raw is None
        else {
            key: [as_bytes(decode_value(item)) for item in list_value(rows, key)]
            for key, rows in object_value(raw, "lists").items()
        }
    )
    lpop_args: list[int] = []
    logger = RecordingLogger()
    cache = object.__new__(RedisCache)
    fields: dict[str, object] = {
        "namespace": input_value.get("namespace"),
        "redis_version": string_value(input_value.get("redis_version"), "redis_version"),
        "_circuit_breaker": RedisCircuitBreaker(failure_threshold=1, recovery_timeout=60),
        "service_logger_obj": logger,
        "init_async_client": lambda: FakeRedis(lists, lpop_args),
    }
    for name, value in fields.items():
        object.__setattr__(cache, name, value)
    result = asyncio.run(drive(input_value, cache))
    return {
        "result": result,
        "lists": project_lists(lists),
        "events": cast(Json, logger.events),
        "lpop_args": cast(Json, lpop_args),
    }
