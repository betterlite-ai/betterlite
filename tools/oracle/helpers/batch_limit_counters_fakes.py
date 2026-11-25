from collections.abc import Mapping
from datetime import UTC, datetime
from types import SimpleNamespace
from typing import ClassVar

from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.hooks.batch_enqueued_tokens import (
    BatchEnqueuedTokenOutcome,
    BatchEnqueuedTokenOverLimit,
    BatchEnqueuedTokenReservation,
    BatchEnqueuedTokenScope,
    BatchEnqueuedTokenStore,
)
from litellm.proxy.hooks.batch_rate_limiter import (
    IncrementAmounts,
)
from litellm.proxy.hooks.batch_rate_limiter import (
    _PROXY_BatchRateLimiter as BatchRateLimiter,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L120
)
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    RateLimitDescriptor,
    RateLimitResponse,
)
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    _PROXY_MaxParallelRequestsHandler_v3 as ParallelLimiter,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/parallel_request_limiter_v3.py#L761
)
from litellm.proxy.utils import InternalUsageCache
from opentelemetry.trace import Span
from pydantic import JsonValue
from typing_extensions import override

from batch_limit_counters_records import (
    LimiterCallView,
    LimiterScript,
    ReplySpec,
    StoreCallView,
    descriptor_view,
    scopes_of,
)


class Trace:
    def __init__(self) -> None:
        self.store_calls: list[StoreCallView] = []
        self.limiter_calls: list[LimiterCallView] = []
        self.cache_reads: list[str] = []
        self.base_calls = 0
        self.project_calls = 0


class FixedClock:
    def __init__(self, at: float) -> None:
        self._at = at

    def __call__(self) -> datetime:
        return datetime.fromtimestamp(self._at, tz=UTC)


class ScriptedCache:
    dual_cache: ClassVar[SimpleNamespace] = SimpleNamespace(redis_cache=None)

    def __init__(self, trace: Trace, values: dict[str, JsonValue]) -> None:
        self._trace = trace
        self._values = values

    async def async_get_cache(
        self,
        *,
        key: str,
        litellm_parent_otel_span: Span | None,
        local_only: bool = False,
    ) -> JsonValue:
        del litellm_parent_otel_span, local_only
        self._trace.cache_reads.append(key)
        return self._values.get(key)


class ScriptedStore:
    def __init__(self, trace: Trace, replies: list[ReplySpec]) -> None:
        self._trace = trace
        self._replies = replies

    async def reserve(
        self,
        tokens: int,
        scopes: tuple[BatchEnqueuedTokenScope, ...],
        litellm_parent_otel_span: Span | None = None,
    ) -> BatchEnqueuedTokenOutcome:
        del litellm_parent_otel_span
        self._trace.store_calls.append({"tokens": tokens, "scopes": scopes_of(scopes)})
        if not self._replies:
            return BatchEnqueuedTokenReservation(tokens=tokens, scopes=scopes)
        reply = self._replies.pop(0)
        if reply["kind"] == "over_limit":
            found = reply.get("scope")
            enqueued = reply.get("enqueued")
            if found is None or enqueued is None:
                raise ValueError("an over-limit reply needs scope and enqueued")
            return BatchEnqueuedTokenOverLimit(
                scope=BatchEnqueuedTokenScope(
                    key=found["key"], value=found["value"], limit=found["limit"]
                ),
                enqueued=enqueued,
            )
        backend = reply.get("backend")
        owner = reply.get("owner")
        return BatchEnqueuedTokenReservation(
            tokens=tokens,
            scopes=scopes,
            backend="memory" if backend is None else backend,
            owner="" if owner is None else owner,
        )


class ScriptedLimiter(ParallelLimiter):
    def __init__(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/parallel_request_limiter_v3.py#L761-L773
        self,
        cache: InternalUsageCache,
        store: BatchEnqueuedTokenStore,
        trace: Trace,
        responses: list[RateLimitResponse],
        window_size: int,
    ) -> None:
        super().__init__(internal_usage_cache=cache, model_group_resolver=no_model_group)
        self.internal_usage_cache = cache
        self.batch_enqueued_token_store = store
        self.window_size = window_size
        self._trace = trace
        self._responses = responses

    @override
    # ast-grep-ignore: no-long-names
    async def atomic_check_and_increment_by_n(
        self,
        descriptors: list[RateLimitDescriptor],
        increments: list[IncrementAmounts],
        parent_otel_span: Span | None = None,
    ) -> RateLimitResponse:
        del parent_otel_span
        self._trace.limiter_calls.append(
            {
                "descriptors": [descriptor_view(descriptor) for descriptor in descriptors],
                "increments": [
                    {"requests": increment["requests"], "tokens": increment["tokens"]}
                    for increment in increments
                ],
            }
        )
        return self._responses.pop(0)


class ScriptedHook(BatchRateLimiter):
    def __init__(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L129-L134
        self,
        cache: InternalUsageCache,
        limiter: ScriptedLimiter,
        clock: float,
        script: LimiterScript,
        trace: Trace,
    ) -> None:
        super().__init__(
            internal_usage_cache=cache,
            parallel_request_limiter=limiter,
            time_provider=FixedClock(clock),
        )
        self._script = script
        self._trace = trace

    @override
    def _create_batch_rate_limit_descriptors(  # ast-grep-ignore: no-long-names
        self,
        user_api_key_dict: UserAPIKeyAuth,
        data: dict[str, JsonValue],
    ) -> list[RateLimitDescriptor]:
        del user_api_key_dict, data
        self._trace.base_calls = self._trace.base_calls + 1
        return self._script["base"]

    @override
    def _create_project_io_descriptors_for_models(  # ast-grep-ignore: no-long-names
        self,
        user_api_key_dict: UserAPIKeyAuth,
        per_model_usage: Mapping[str, Mapping[str, int]],
    ) -> tuple[list[RateLimitDescriptor], list[IncrementAmounts]]:
        del user_api_key_dict, per_model_usage
        self._trace.project_calls = self._trace.project_calls + 1
        project = self._script["project"]
        return project["descriptors"], project["increments"]


def no_model_group(_model: str) -> str | None:  # ast-grep-ignore: no-one-line-functions
    return None
