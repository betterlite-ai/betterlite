import asyncio
import hashlib
from collections.abc import Mapping, Sequence
from datetime import timedelta
from typing import Self, TypeAlias, TypeVar, cast

from litellm.caching.redis_batch import BatchResult, RedisBatch, RegisteredScript
from litellm.caching.redis_cache import (
    RedisCache,
    RedisCircuitBreaker,
    RedisCircuitBreakerOpenError,
)
from litellm.caching.redis_cluster_cache import RedisClusterCache
from redis.exceptions import ConnectionError as RedisConnectionError
from redis.exceptions import NoScriptError, ResponseError

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Declared: TypeAlias = (
    BatchResult[Mapping[str, object]] | BatchResult[object] | BatchResult[float] | BatchResult[None]
)
_T = TypeVar("_T")

REPORTED: frozenset[str] = frozenset(
    {"MGET", "EVALSHA", "SCRIPT", "INCRBYFLOAT", "EXPIRE", "SET", "DEL", "TTL"}
)


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def list_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def integer_value(value: Json, name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{name} must be an integer")
    return value


def optional_integer(value: Json, name: str) -> int | None:
    if value is None:
        return None
    return integer_value(value, name)


def number_value(value: Json, name: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be a number")
    return float(value)


def optional_number(value: Json, name: str) -> float | None:
    if value is None:
        return None
    return number_value(value, name)


def optional_string(value: Json, name: str) -> str | None:
    if value is None:
        return None
    return string_value(value, name)


def reply_value(spec: Json) -> object:
    if spec == "nil":
        return None
    entry = object_value(spec, "reply")
    if "bulk" in entry:
        return string_value(entry["bulk"], "bulk")
    if "simple" in entry:
        return string_value(entry["simple"], "simple")
    if "int" in entry:
        return integer_value(entry["int"], "int")
    if "error" in entry:
        message = string_value(entry["error"], "error")
        return NoScriptError(message) if message.startswith("NOSCRIPT") else ResponseError(message)
    if "array" in entry:
        return [reply_value(item) for item in list_value(entry["array"], "array")]
    raise ValueError("unknown reply spec")


def arg_value(value: Json) -> str | int | float:
    entry = object_value(value, "arg")
    if "text" in entry:
        return string_value(entry["text"], "text")
    if "int" in entry:
        return integer_value(entry["int"], "int")
    return number_value(entry["float"], "float")


class ScriptRunner:
    def __init__(self, client: "FakeClient", script: str) -> None:
        self._client = client
        self._script = script
        # EVALSHA identifies scripts by SHA-1
        self._sha = hashlib.sha1(script.encode()).hexdigest()  # noqa: S324

    async def __call__(
        self, keys: tuple[str, ...], args: tuple[object, ...], client: object = None
    ) -> object:
        _ = client
        command = [self._sha, str(len(keys)), *keys, *(str(arg) for arg in args)]
        reply = self._client.reply("EVALSHA", command)
        if isinstance(reply, NoScriptError):
            self._client.reply("SCRIPT", ["LOAD", self._script])
            reply = self._client.reply("EVALSHA", command)
        return reply


class FakePipeline:
    def __init__(self, client: "FakeClient") -> None:
        self._client = client
        self._queued: list[tuple[str, list[str]]] = []

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(self, kind: object, value: object, traceback: object) -> None:
        _ = (kind, value, traceback)

    def mget(self, keys: tuple[str, ...]) -> None:
        self._queued.append(("MGET", list(keys)))

    def evalsha(self, sha: str, numkeys: int, *keys_and_args: object) -> None:
        self._queued.append(("EVALSHA", [sha, str(numkeys), *(str(arg) for arg in keys_and_args)]))

    def incrbyfloat(self, name: str, amount: float) -> None:
        self._queued.append(("INCRBYFLOAT", [name, str(amount)]))

    def ttl(self, name: str) -> None:
        self._queued.append(("TTL", [name]))

    def expire(self, name: str, time: timedelta) -> None:
        self._queued.append(("EXPIRE", [name, str(int(time.total_seconds()))]))

    def set(self, name: str, value: str, ex: timedelta | None = None) -> None:
        command = [name, value]
        if ex is not None:
            command.extend(["EX", str(int(ex.total_seconds()))])
        self._queued.append(("SET", command))

    def delete(self, *names: str) -> None:
        self._queued.append(("DEL", [*names]))

    async def execute(self, *, raise_on_error: bool = True) -> list[object]:
        results: list[object] = []
        for name, args in self._queued:
            result = self._client.reply(name, args)
            if isinstance(result, Exception) and raise_on_error:
                raise result
            results.append(result)
        self._queued.clear()
        return results


class FakeClient:
    def __init__(self, replies: dict[str, list[Json]], *, refused: bool) -> None:
        self.replies = replies
        self.commands: list[list[str]] = []
        self.refused = refused

    def init_async_client(self) -> "FakeClient":
        if self.refused:
            raise RedisConnectionError("connection refused")
        return self

    def pipeline(self, *, transaction: bool = False) -> FakePipeline:
        _ = transaction
        return FakePipeline(self)

    def reply(self, name: str, args: list[str]) -> object:
        self.commands.append([name, *args])
        queue = self.replies.get(name.upper())
        spec: Json = queue.pop(0) if queue else {"simple": "OK"}
        return reply_value(spec)

    async def mget(self, keys: list[str]) -> list[object]:
        return cast(list[object], self.reply("MGET", list(keys)))

    async def mget_nonatomic(self, keys: list[str]) -> list[object]:
        return cast(list[object], self.reply("MGET", list(keys)))

    async def incrbyfloat(self, name: str, amount: float) -> float:
        return cast(float, self.reply("INCRBYFLOAT", [name, str(amount)]))

    async def ttl(self, name: str) -> int:
        return cast(int, self.reply("TTL", [name]))

    async def expire(self, name: str, time: int) -> bool:
        return cast(bool, self.reply("EXPIRE", [name, str(time)]))

    async def delete(self, name: str) -> int:
        return cast(int, self.reply("DEL", [name]))

    def register_script(self, script: str) -> ScriptRunner:
        return ScriptRunner(self, script)


class Logger:
    async def async_service_success_hook(self, **_: object) -> None:
        return None

    async def async_service_failure_hook(self, **_: object) -> None:
        return None

    def service_success_hook(self, **_: object) -> None:
        return None

    def service_failure_hook(self, **_: object) -> None:
        return None


def build_cache(call: dict[str, Json], client: FakeClient) -> RedisCache:
    cache = cast(
        RedisCache, object.__new__(RedisClusterCache if call.get("cluster") is True else RedisCache)
    )
    fields: dict[str, object] = {
        "namespace": optional_string(call.get("namespace"), "namespace"),
        "default_ttl": optional_integer(call.get("default_ttl"), "default_ttl"),
        "redis_kwargs": {},
        "_circuit_breaker": RedisCircuitBreaker(failure_threshold=1, recovery_timeout=60),
        "service_logger_obj": Logger(),
        "init_async_client": client.init_async_client,
    }
    for name, value in fields.items():
        object.__setattr__(cache, name, value)
    return cache


def project(value: object) -> Json:
    if isinstance(value, bytes):
        try:
            return value.decode()
        except UnicodeDecodeError:
            return list(value)
    if isinstance(value, (list, tuple)):
        return project_list(cast("Sequence[object]", value))
    if isinstance(value, Mapping):
        return project_dict(cast("Mapping[object, object]", value))
    if value is None or isinstance(value, (bool, int, float, str)):
        return cast(Json, value)
    raise ValueError(f"unsupported value {type(value).__name__}")


def project_list(items: Sequence[object]) -> Json:
    return [project(item) for item in items]


def project_dict(entries: Mapping[object, object]) -> Json:
    return {string_value(cast(Json, key), "key"): project(item) for key, item in entries.items()}


def taxonomy(error: BaseException) -> str:
    if isinstance(error, NoScriptError):
        return "noscript"
    if isinstance(error, RedisCircuitBreakerOpenError):
        return "breaker"
    if isinstance(error, ResponseError):
        return "reply"
    if isinstance(error, RedisConnectionError):
        return "connection"
    if isinstance(error, (TypeError, ValueError, KeyError, IndexError)):
        return "reply"
    return "other"


def declare(
    batch: RedisBatch, cache: RedisCache, spec: dict[str, Json], index: int, log: list[Json]
) -> Declared:
    kind = string_value(spec.get("kind"), "kind")
    match kind:
        case "mget":
            keys = [string_value(key, "key") for key in list_value(spec.get("keys"), "keys")]
            result = batch.mget(keys)
        case "script":
            source = string_value(spec.get("source"), "source")
            keys = [string_value(key, "key") for key in list_value(spec.get("keys"), "keys")]
            args = [arg_value(arg) for arg in list_value(spec.get("args"), "args")]
            run = cast(RegisteredScript, cache.async_register_script(source))
            result = batch.script(source, run, keys, args)
        case "increment":
            result = batch.increment(
                string_value(spec.get("key"), "key"),
                number_value(spec.get("value"), "value"),
                optional_integer(spec.get("ttl"), "ttl"),
            )
        case "set":
            result = batch.set(
                string_value(spec.get("key"), "key"),
                spec.get("value"),
                optional_number(spec.get("ttl"), "ttl"),
            )
        case "delete":
            result = batch.delete(string_value(spec.get("key"), "key"))
        case _:
            raise ValueError(f"unknown operation {kind}")

    def hook(future: asyncio.Future[_T]) -> None:
        if future.cancelled():
            status = "cancelled"
        elif future.exception() is not None:
            status = "failed"
        else:
            status = "completed"
        log.append({"handle": index, "status": status})

    result.on_settled(hook)
    return result


class Walk:
    def __init__(self, batch: RedisBatch, cache: RedisCache, log: list[Json]) -> None:
        self.batch = batch
        self.cache = cache
        self.log = log
        self.handles: list[Declared | None] = []
        self.results: list[Json] = []
        self.misses: list[Json] = []

    async def step(self, step: Json) -> None:
        if isinstance(step, str):
            await self.named(step)
            return
        entry = object_value(step, "step")
        if "declare" in entry:
            operation = object_value(entry["declare"], "declare")
            index = len(self.handles)
            self.handles.append(declare(self.batch, self.cache, operation, index, self.log))
        elif "await" in entry:
            await self.take_at(integer_value(entry["await"], "await"))
        elif "missing" in entry:
            key = string_value(entry["missing"], "missing")
            self.misses.append({"key": key, "missing": self.batch.read_as_missing(key)})
        else:
            raise ValueError("unknown step")

    async def named(self, step: str) -> None:
        match step:
            case "await_all":
                for index in range(len(self.handles)):
                    await self.take_at(index)
            case "flush":
                await self.batch.flush()
            case _:
                raise ValueError(f"unknown step {step}")

    async def take_at(self, index: int) -> None:
        handle = self.handles[index]
        if handle is None:
            return
        self.handles[index] = None
        self.results.append(await take(handle, index))


async def take(handle: Declared, index: int) -> Json:
    try:
        value = await handle
    except Exception as error:  # noqa: BLE001 - the wire contract carries every exception
        return {"handle": index, "ok": False, "error": taxonomy(error)}
    return {"handle": index, "ok": True, "value": project(value)}


async def drive(call: dict[str, Json]) -> Json:
    raw_replies = object_value(call.get("replies") or {}, "replies")
    replies = {name: list_value(rows, name) for name, rows in raw_replies.items()}
    client = FakeClient(replies, refused=call.get("refused") is True)
    cache = build_cache(call, client)
    batch = RedisBatch(redis_cache=cache, name="redis_batch")
    log: list[Json] = []
    walk = Walk(batch, cache, log)
    for step in list_value(call.get("steps"), "steps"):
        await walk.step(step)
    await batch.flush()
    return cast(
        Json,
        {
            "results": walk.results,
            "settled": log,
            "misses": walk.misses,
            "commands": [command for command in client.commands if command[0] in REPORTED],
            "flushes": batch.flushes,
            "pending": batch.pending,
        },
    )


def run(input_value: dict[str, Json]) -> Json:
    return asyncio.run(drive(input_value))
