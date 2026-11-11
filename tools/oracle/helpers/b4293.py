import asyncio
import json
import socket
import socketserver
import threading
from collections import defaultdict, deque
from collections.abc import Awaitable, Callable
from concurrent.futures import ThreadPoolExecutor
from functools import partial
from typing import NamedTuple, Protocol, TypeAlias, TypedDict, cast

from litellm.caching.dual_cache import DualCache
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.caching.redis_batch import active_request_redis_batch, request_redis_batch_scope
from litellm.caching.redis_cache import RedisCache
from typing_extensions import override

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Pending: TypeAlias = Awaitable[Json] | None

NIL = b"$-1\r\n"
FAILURES = {
    "increment_propagates_redis_error": "INCRBY",
    "async_increment_fallback_on_redis_error": "INCRBYFLOAT",
}


def bulk(text: str) -> bytes:
    data = text.encode()
    return b"$%d\r\n%s\r\n" % (len(data), data)


FALLBACKS = {
    "PING": b"+PONG\r\n",
    "INFO": bulk("redis_version:7.2.0\r\n"),
    "GET": NIL,
    "INCRBYFLOAT": bulk("1.0"),
    "TTL": b":60\r\n",
    **dict.fromkeys(["INCRBY", "DEL", "EXPIRE", "SADD"], b":1\r\n"),
}


class Entry(TypedDict):
    key: str
    value: Json


class Fields(TypedDict):
    clock: float
    key: str
    value: Json
    redis_get: Json
    memory: list[Entry]
    default_in_memory_ttl: float | None
    entries: list[Entry]
    key_count: int


class TypedCache(Protocol):
    def get_cache(self, key: str, *, local_only: bool = False) -> Json: ...
    def set_cache(self, key: str, value: Json, *, local_only: bool = False) -> None: ...
    def increment_cache(self, key: str, value: Json) -> Json: ...
    async def async_get_cache(self, key: str) -> Json: ...
    async def async_get_ttl(self, key: str) -> int | None: ...
    async def async_increment_cache(self, key: str, value: Json) -> Json: ...
    async def async_set_cache_pre_call(self, key: str, value: Json, ttl: None) -> Pending: ...
    async def async_set_cache_pipeline(self, cache_list: list[tuple[str, Json]]) -> None: ...
    async def async_delete_cache_keys(self, keys: list[str]) -> None: ...


class Script:
    def __init__(self) -> None:
        self.replies: defaultdict[str, deque[bytes]] = defaultdict(deque)
        self.received: list[list[str]] = []
        self.lock = threading.Lock()

    def answer(self, args: list[str]) -> bytes:
        name = args[0].upper()
        with self.lock:
            self.received.append(args)
            scripted = self.replies[name]
            if scripted:
                return scripted.popleft()
        if name == "MGET":
            return b"*%d\r\n%s" % (len(args) - 1, NIL * (len(args) - 1))
        fallback = FALLBACKS.get(name)
        return b"+OK\r\n" if fallback is None else fallback

    def expiries(self) -> list[Json]:
        with self.lock:
            sets = [args for args in self.received if args[0].upper() == "SET"]
        return [{"key": s[1], "ex": s[s.index("EX") + 1] if "EX" in s else ""} for s in sets]


class Handler(socketserver.StreamRequestHandler):
    def __init__(
        self,
        script: Script,
        request: socket.socket,
        client_address: tuple[str, int],
        server: socketserver.BaseServer,
    ) -> None:
        self.script = script
        super().__init__(request, client_address, server)

    @override
    def handle(self) -> None:
        header = self.rfile.readline()
        while header:
            args: list[str] = []
            for _ in range(int(header[1:])):
                size = int(self.rfile.readline()[1:])
                args.append(self.rfile.read(size + 2)[:size].decode())
            self.wfile.write(self.script.answer(args))
            header = self.rfile.readline()


class Harness(NamedTuple):
    dual: TypedCache
    redis: RedisCache
    script: Script


Scenario: TypeAlias = Callable[[Fields, Harness], Awaitable[dict[str, Json]]]


async def observe(harness: Harness, key: str, read: Json) -> dict[str, Json]:
    ttl = await harness.dual.async_get_ttl(key)
    return {"read": read, "ttl": ttl, "second": harness.dual.get_cache(key)}


async def read(call: Fields, harness: Harness) -> dict[str, Json]:
    for entry in call["memory"]:
        harness.dual.set_cache(entry["key"], entry["value"], local_only=True)
    found = harness.dual.get_cache(call["key"])
    return await observe(harness, call["key"], found)


async def batch_read(call: Fields, harness: Harness) -> dict[str, Json]:
    with request_redis_batch_scope(post_call_deadline=1.0):
        batch = active_request_redis_batch(harness.redis)
        if batch is None:
            raise RuntimeError("the request scope has no Redis batch")
        await batch.mget([call["key"]])
        found = await harness.dual.async_get_cache(call["key"])
    return await observe(harness, call["key"], found)


async def set_default_ttl(call: Fields, harness: Harness) -> dict[str, Json]:
    harness.dual.set_cache(call["key"], call["value"])
    ttl = await harness.dual.async_get_ttl(call["key"])
    return {"memory_ttl": ttl, "sets": harness.script.expiries()}


async def increment_error(call: Fields, harness: Harness) -> dict[str, Json]:
    with ThreadPoolExecutor(max_workers=1) as pool:
        failure = pool.submit(harness.dual.increment_cache, call["key"], call["value"]).exception()
    memory = harness.dual.get_cache(call["key"], local_only=True)
    return {"error": failure is not None, "memory": memory}


async def increment_fallback(call: Fields, harness: Harness) -> dict[str, Json]:
    result = await harness.dual.async_increment_cache(call["key"], call["value"])
    return {"result": result}


async def pre_call(call: Fields, harness: Harness) -> dict[str, Json]:
    key, value = call["key"], call["value"]
    outside = await harness.dual.async_set_cache_pre_call(key, value, None)
    in_scope = False
    with request_redis_batch_scope(post_call_deadline=1.0):
        queued = await harness.dual.async_set_cache_pre_call(key, value, None)
        if queued is not None:
            in_scope = True
            await queued
    memory = harness.dual.get_cache(key, local_only=True)
    sets = harness.script.expiries()
    return {"outside": outside is not None, "in_scope": in_scope, "memory": memory, "sets": sets}


async def pipeline(call: Fields, harness: Harness) -> dict[str, Json]:
    pairs = [(entry["key"], entry["value"]) for entry in call["entries"]]
    await harness.dual.async_set_cache_pipeline(pairs)
    ttls: list[Json] = [await harness.dual.async_get_ttl(key) for key, _value in pairs]
    return {"ttls": ttls, "sets": harness.script.expiries()}


async def delete_keys(call: Fields, harness: Harness) -> dict[str, Json]:
    keys = [f"key-{index}" for index in range(call["key_count"])]
    for key in keys:
        harness.dual.set_cache(key, "value", local_only=True)
    await harness.dual.async_delete_cache_keys(keys)
    remaining = 0
    for key in keys:
        remaining += harness.dual.get_cache(key, local_only=True) is not None
    received = harness.script.received
    chunks: list[Json] = [len(args) - 1 for args in received if args[0].upper() == "DEL"]
    return {"chunks": chunks, "remaining": remaining}


SCENARIOS: dict[str, Scenario] = {
    "get_hits_memory_first": read,
    "get_backfills_memory_from_redis": read,
    "async_get_checks_request_batch_missing": batch_read,
    "set_cache_applies_default_ttl": set_default_ttl,
    "increment_propagates_redis_error": increment_error,
    "async_increment_fallback_on_redis_error": increment_fallback,
    "set_pre_call_returns_batch_or_none": pre_call,
    "set_pipeline_applies_single_ttl": pipeline,
    "delete_cache_keys_chunks_redis": delete_keys,
}


async def execute(name: str, call: Fields, port: int, script: Script) -> dict[str, Json]:
    redis = RedisCache(host="127.0.0.1", port=port, socket_timeout=2.0)
    dual = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/dual_cache.py#L164
        TypedCache,
        DualCache(
            in_memory_cache=InMemoryCache(clock=partial(float, call["clock"])),
            redis_cache=redis,
            default_in_memory_ttl=call["default_in_memory_ttl"],
            default_redis_ttl=None,
            default_redis_batch_cache_expiry=None,
        ),
    )
    return await SCENARIOS[name](call, Harness(dual, redis, script))


def run(input_value: dict[str, Fields]) -> dict[str, Json]:
    ((name, call),) = input_value.items()
    script = Script()
    script.replies["GET"].append(bulk(json.dumps(call["redis_get"])))
    if name in FAILURES:
        script.replies[FAILURES[name]].append(b"-ERR boom\r\n")
    with socketserver.ThreadingTCPServer(("127.0.0.1", 0), partial(Handler, script)) as server:
        server.daemon_threads = True
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            return asyncio.run(execute(name, call, server.server_address[1], script))
        finally:
            server.shutdown()
