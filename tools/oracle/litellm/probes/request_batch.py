import asyncio
from typing import cast

from litellm.caching.redis_batch import (
    active_request_redis_batch,
    active_request_redis_batches,
    request_redis_batch_scope,
)
from litellm.caching.redis_cache import RedisCache
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true


class Output(Strict):
    shared: bool
    nested: bool
    taken: str | None


def cache(namespace: str) -> RedisCache:
    built = object.__new__(RedisCache)
    object.__setattr__(built, "namespace", namespace)
    object.__setattr__(built, "redis_kwargs", {"host": "one", "port": 6379})
    return built


class RequestBatch(Operation):
    namespace: str
    slot: str
    value: str

    @override
    def run(self) -> Output:
        return asyncio.run(self.execute())

    async def execute(self) -> Output:
        first = cache(self.namespace)
        with request_redis_batch_scope() as request:
            batch = active_request_redis_batch(first)
            twin = active_request_redis_batch(cache(self.namespace))
            outside = active_request_redis_batches()
            with request_redis_batch_scope() as nested:
                inside = active_request_redis_batches()
            request.prefetched[self.slot] = self.value
            taken = request.prefetched.pop(self.slot, None)
        return Output(
            shared=batch is not None and batch is twin,
            nested=outside is inside and nested is request,
            taken=cast("str | None", taken),
        )
