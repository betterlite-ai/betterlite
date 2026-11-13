import asyncio
from typing import Self, cast

from litellm.caching.redis_batch import RedisBatch
from litellm.caching.redis_cache import RedisCache, RedisCircuitBreaker
from pydantic import JsonValue
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true


class Output(Strict):
    results: list[JsonValue]


class Pipeline:
    def __init__(self) -> None:
        self.queued: list[tuple[str, list[str]]] = []

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(self, kind: object, value: object, traceback: object) -> None:
        _ = (kind, value, traceback)

    def mget(self, keys: list[str]) -> None:
        self.queued.append(("MGET", list(keys)))

    def incrbyfloat(self, name: str, amount: float) -> None:
        self.queued.append(("INCRBYFLOAT", [name, str(amount)]))

    def expire(self, name: str, time: int) -> None:
        self.queued.append(("EXPIRE", [name, str(time)]))

    async def execute(self, *, raise_on_error: bool = True) -> list[object]:
        _ = raise_on_error
        replies: list[object] = []
        for name, _args in self.queued:
            if name == "MGET":
                replies.append(["1"])
            elif name == "INCRBYFLOAT":
                replies.append("1.5")
            else:
                replies.append(1)
        self.queued.clear()
        return replies


class Client:
    def pipeline(self, *, transaction: bool = False) -> Pipeline:
        _ = transaction
        return Pipeline()


class Logger:
    async def async_service_success_hook(self, **_: object) -> None:
        return None

    async def async_service_failure_hook(self, **_: object) -> None:
        return None

    def service_success_hook(self, **_: object) -> None:
        return None

    def service_failure_hook(self, **_: object) -> None:
        return None


def fake_client() -> Client:
    return Client()


class Batch(Operation):
    namespace: str | None = None

    @override
    def run(self) -> Output:
        return Output(results=asyncio.run(self.execute()))

    async def execute(self) -> list[JsonValue]:
        cache = object.__new__(RedisCache)
        fields: dict[str, object] = {
            "namespace": self.namespace,
            "default_ttl": 60,
            "_circuit_breaker": RedisCircuitBreaker(failure_threshold=1, recovery_timeout=60),
            "service_logger_obj": Logger(),
            "init_async_client": fake_client,
        }
        for name, value in fields.items():
            object.__setattr__(cache, name, value)
        batch = RedisBatch(redis_cache=cache)
        mget = batch.mget(["alpha"])
        increment = batch.increment("counter", 1.5, 20)
        return [
            {"operation": "mget", "value": cast("JsonValue", dict(await mget))},
            {"operation": "increment", "value": await increment},
        ]
