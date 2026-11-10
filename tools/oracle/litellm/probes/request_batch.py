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
    same_settings_share: bool
    different_namespace_separate: bool
    nested_request_shares: bool
    nested_batches_join: bool
    prefetch_armed: bool
    prefetch_taken: str | None
    prefetch_after_take: bool


def build_cache(namespace: str, host: str) -> RedisCache:
    cache = object.__new__(RedisCache)
    object.__setattr__(cache, "namespace", namespace)
    object.__setattr__(cache, "redis_kwargs", {"host": host, "port": 6379})
    return cache


class RequestBatch(Operation):
    namespace: str
    other_namespace: str
    slot: str
    value: str

    @override
    def run(self) -> Output:
        return asyncio.run(self.execute())

    async def execute(self) -> Output:
        first = build_cache(self.namespace, "one")
        twin = build_cache(self.namespace, "one")
        other = build_cache(self.other_namespace, "one")
        with request_redis_batch_scope() as request:
            batch = active_request_redis_batch(first)
            twin_batch = active_request_redis_batch(twin)
            other_batch = active_request_redis_batch(other)
            outside = active_request_redis_batches()
            with request_redis_batch_scope() as nested:
                inside = active_request_redis_batches()
                nested_batch = active_request_redis_batch(twin)
            request.prefetched[self.slot] = self.value
            armed = self.slot in request.prefetched
            taken = request.prefetched.pop(self.slot, None)
            after_take = self.slot in request.prefetched
        return Output(
            same_settings_share=batch is not None and batch is twin_batch,
            different_namespace_separate=batch is not other_batch,
            nested_request_shares=outside is inside and nested is request,
            nested_batches_join=batch is not None and batch is nested_batch,
            prefetch_armed=armed,
            prefetch_taken=cast("str | None", taken),
            prefetch_after_take=after_take,
        )
