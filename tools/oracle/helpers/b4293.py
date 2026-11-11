import asyncio
import json
import socket
import socketserver
import threading
from collections import defaultdict, deque
from collections.abc import Awaitable, Callable
from concurrent.futures import ThreadPoolExecutor
from functools import partial
from typing import NamedTuple, NotRequired, TypeAlias, TypedDict, cast

from litellm.caching.dual_cache import DualCache
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.caching.redis_batch import active_request_redis_batch, request_redis_batch_scope
from litellm.caching.redis_cache import RedisCache
from typing_extensions import override

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

DEADLINE_SECONDS = 1.0
SOCKET_TIMEOUT_SECONDS = 2.0
OK = b"+OK\r\n"
NIL = b"$-1\r\n"
ERROR = b"-ERR boom\r\n"
# the command whose first reply fails, per scenario
FAILURES = {
    "increment_propagates_redis_error": "INCRBY",
    "async_increment_fallback_on_redis_error": "INCRBYFLOAT",
}


def bulk(text: str) -> bytes:
    data = text.encode()
    return b"$%d\r\n%s\r\n" % (len(data), data)


# what the fake Redis answers once a command has no scripted reply left
FALLBACKS = {
    "PING": b"+PONG\r\n",
    "INFO": bulk("redis_version:7.2.0\r\n"),
    "GET": NIL,
    "INCRBYFLOAT": bulk("1.0"),
    "TTL": b":60\r\n",
    "INCRBY": b":1\r\n",
    "DEL": b":1\r\n",
    "EXPIRE": b":1\r\n",
    "SADD": b":1\r\n",
}


class Entry(TypedDict):
    key: str
    value: Json


class Fields(TypedDict):
    clock: float
    key: NotRequired[str]
    value: NotRequired[Json]
    redis_get: NotRequired[Json]
    memory: NotRequired[list[Entry]]
    default_in_memory_ttl: NotRequired[float]
    entries: NotRequired[list[Entry]]


def expiry(args: list[str]) -> str:
    if "EX" not in args:
        return ""
    return args[args.index("EX") + 1]


class Script:
    def __init__(self, replies: defaultdict[str, deque[bytes]]) -> None:
        self.replies = replies
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
        return OK if fallback is None else fallback

    def expiries(self) -> list[Json]:
        with self.lock:
            sets = [args for args in self.received if args[0].upper() == "SET"]
        return [{"key": args[1], "ex": expiry(args)} for args in sets]


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


class Clock:
    def __init__(self, now: float) -> None:
        self.now = now

    def __call__(self) -> float:
        return self.now


class Harness(NamedTuple):
    dual: DualCache
    redis: RedisCache
    script: Script


Scenario: TypeAlias = Callable[[Fields, Harness], Awaitable[dict[str, Json]]]


async def observe(harness: Harness, key: str, read: Json) -> dict[str, Json]:
    ttl: Json = await harness.dual.async_get_ttl(key)
    second: Json = harness.dual.get_cache(key)
    return {"read": read, "ttl": ttl, "second": second}


async def read(call: Fields, harness: Harness) -> dict[str, Json]:
    memory = call.get("memory")
    if memory is not None:
        for entry in memory:
            harness.dual.set_cache(entry["key"], entry["value"], local_only=True)
    found: Json = harness.dual.get_cache(call["key"])
    return await observe(harness, call["key"], found)


async def batch_read(call: Fields, harness: Harness) -> dict[str, Json]:
    with request_redis_batch_scope(post_call_deadline=DEADLINE_SECONDS):
        batch = active_request_redis_batch(harness.redis)
        if batch is None:
            raise RuntimeError("the request scope has no Redis batch")
        await batch.mget([call["key"]])
        found = await cast(Awaitable[Json], harness.dual.async_get_cache(call["key"]))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/dual_cache.py#L261
    return await observe(harness, call["key"], found)


async def set_default_ttl(call: Fields, harness: Harness) -> dict[str, Json]:
    harness.dual.set_cache(call["key"], call["value"])
    ttl: Json = await harness.dual.async_get_ttl(call["key"])
    return {"memory_ttl": ttl, "sets": harness.script.expiries()}


async def increment_error(call: Fields, harness: Harness) -> dict[str, Json]:
    amount = cast(int, call["value"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/dual_cache.py#L164
    increment = cast(Callable[[str, int], int], harness.dual.increment_cache)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/dual_cache.py#L164
    with ThreadPoolExecutor(max_workers=1) as pool:
        failure = pool.submit(increment, call["key"], amount).exception()
    memory: Json = harness.dual.get_cache(call["key"], local_only=True)
    return {"error": failure is not None, "memory": memory}


async def increment_fallback(call: Fields, harness: Harness) -> dict[str, Json]:
    amount = cast(float, call["value"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/dual_cache.py#L565
    result: Json = await harness.dual.async_increment_cache(call["key"], amount)
    return {"result": result}


async def pre_call(call: Fields, harness: Harness) -> dict[str, Json]:
    key, value = call["key"], call["value"]
    outside = await harness.dual.async_set_cache_pre_call(key, value, None)
    in_scope = False
    with request_redis_batch_scope(post_call_deadline=DEADLINE_SECONDS):
        queued = await harness.dual.async_set_cache_pre_call(key, value, None)
        if queued is not None:
            in_scope = True
            await queued
    memory: Json = harness.dual.get_cache(key, local_only=True)
    sets = harness.script.expiries()
    return {"outside": outside is not None, "in_scope": in_scope, "memory": memory, "sets": sets}


async def pipeline(call: Fields, harness: Harness) -> dict[str, Json]:
    pairs = [(entry["key"], entry["value"]) for entry in call["entries"]]
    await cast(Awaitable[None], harness.dual.async_set_cache_pipeline(pairs))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/dual_cache.py#L543
    ttls: list[Json] = [await harness.dual.async_get_ttl(key) for key, _value in pairs]
    return {"ttls": ttls, "sets": harness.script.expiries()}


SCENARIOS: dict[str, Scenario] = {
    "get_hits_memory_first": read,
    "get_backfills_memory_from_redis": read,
    "async_get_checks_request_batch_missing": batch_read,
    "set_cache_applies_default_ttl": set_default_ttl,
    "increment_propagates_redis_error": increment_error,
    "async_increment_fallback_on_redis_error": increment_fallback,
    "set_pre_call_returns_batch_or_none": pre_call,
    "set_pipeline_applies_single_ttl": pipeline,
}


async def execute(name: str, call: Fields, port: int, script: Script) -> dict[str, Json]:
    redis = RedisCache(host="127.0.0.1", port=port, socket_timeout=SOCKET_TIMEOUT_SECONDS)
    dual = DualCache(
        in_memory_cache=InMemoryCache(clock=Clock(call["clock"])),
        redis_cache=redis,
        default_in_memory_ttl=call.get("default_in_memory_ttl"),
        default_redis_ttl=None,
        default_redis_batch_cache_expiry=None,
    )
    return await SCENARIOS[name](call, Harness(dual, redis, script))


def run(input_value: dict[str, Fields]) -> dict[str, Json]:
    ((name, call),) = input_value.items()
    replies: defaultdict[str, deque[bytes]] = defaultdict(deque)
    replies["GET"].append(bulk(json.dumps(call.get("redis_get"))))
    if name in FAILURES:
        replies[FAILURES[name]].append(ERROR)
    script = Script(replies)
    with socketserver.ThreadingTCPServer(("127.0.0.1", 0), partial(Handler, script)) as server:
        server.daemon_threads = True
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            return asyncio.run(execute(name, call, server.server_address[1], script))
        finally:
            server.shutdown()
