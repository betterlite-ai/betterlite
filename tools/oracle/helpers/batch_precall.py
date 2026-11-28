import asyncio
import contextlib
import io
import logging
from collections.abc import Sequence
from typing import Final, Literal, TypedDict, cast

import litellm
from litellm.caching.caching import Cache
from litellm.caching.dual_cache import DualCache
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.caching.redis_cache import RedisCache
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.hooks.batch_enqueued_tokens import (
    BatchEnqueuedTokenReservation,
    BatchEnqueuedTokenScope,
    BatchEnqueuedTokenStore,
)
from litellm.proxy.hooks.batch_rate_limiter import BatchFileUsage
from litellm.proxy.hooks.batch_rate_limiter import (
    _PROXY_BatchRateLimiter as BatchRateLimiter,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L120
)
from litellm.proxy.hooks.batch_redis_get import (
    _PROXY_BatchRedisRequests as BatchRedisRequests,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_redis_get.py#L18
)
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    RateLimitDescriptor,
    get_or_create_request_stash,
)
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    _PROXY_MaxParallelRequestsHandler_v3 as Limiter,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/parallel_request_limiter_v3.py#L751
)
from litellm.proxy.utils import InternalUsageCache
from litellm.types.caching import LiteLLMCacheType
from opentelemetry.trace import Span
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

JSON: Final = TypeAdapter(JsonValue)
BATCH_USAGE: Final = TypeAdapter(BatchFileUsage)
RESERVATION: Final = TypeAdapter(BatchEnqueuedTokenReservation)
REDACTED: Final = "REDACTED"


class StagesJson(TypedDict):
    skip: bool
    usage: JsonValue
    backend: Literal["redis", "memory"] | None
    owner: str


class CallJson(TypedDict):
    op: Literal["create", "create_failure", "retrieve", "terminal_retrieve"]
    call_type: str
    principal: dict[str, JsonValue]
    data: dict[str, JsonValue]
    cache_key: str
    batch_id: str
    status: str


class Input(TypedDict):
    call: CallJson
    stages: StagesJson
    stash: dict[str, JsonValue]
    memory: dict[str, JsonValue]
    remote: dict[str, JsonValue]


class Sink(logging.Handler):
    def __init__(self, api_key: str | None) -> None:
        super().__init__(level=logging.DEBUG)
        self._api_key = api_key
        self.lines: list[JsonValue] = []
        logging.getLogger("LiteLLM Proxy").setLevel(logging.DEBUG)
        logging.getLogger("LiteLLM Proxy").addHandler(self)

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        api_key = self._api_key
        if api_key is not None:
            # the same redaction betterlite applies, so the rest of each line still compares
            message = message.replace(api_key, REDACTED)
        if not message.startswith("Traceback (most recent call last)"):
            self.lines.append({"level": record.levelname.lower(), "message": message})


def reservation_view(reservation: BatchEnqueuedTokenReservation) -> JsonValue:
    dumped = RESERVATION.dump_json(reservation, exclude={"reserved_at_monotonic"})
    return JSON.validate_json(dumped)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_enqueued_tokens.py#L87


class ScriptedRedis(RedisCache):
    def __init__(self, spec: dict[str, JsonValue], effects: list[JsonValue]) -> None:  # pyright: ignore[reportMissingSuperCall]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/redis_cache.py#L604
        self._spec = spec
        self._effects = effects

    @override
    async def async_scan_iter(self, pattern: str, count: int = 100) -> list[str]:
        del pattern, count
        self._effects.append("scan")
        faults = cast(list[dict[str, JsonValue]], self._spec["faults"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/redis_cache.py#L946
        if faults:
            raise RuntimeError(str(faults[0]["message"]))
        return cast(list[str], self._spec["scan_keys"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/redis_cache.py#L946

    @override
    async def async_batch_get_cache(
        self, key_list: list[str] | list[str | None], parent_otel_span: Span | None = None
    ) -> dict[str, JsonValue]:
        del key_list, parent_otel_span
        self._effects.append("batch_get")
        return dict(cast(dict[str, JsonValue], self._spec["batch_entries"]))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/redis_cache.py#L1696

    @override
    async def async_get_cache(
        self, key: str, parent_otel_span: Span | None = None, **kwargs: JsonValue
    ) -> JsonValue:
        del key, parent_otel_span, kwargs
        self._effects.append("get")
        return self._spec["get_value"]


class ScriptedStore(BatchEnqueuedTokenStore):
    def __init__(self, stages: StagesJson, effects: list[JsonValue]) -> None:
        super().__init__(internal_usage_cache=InternalUsageCache(DualCache()))
        self._stages = stages
        self._effects = effects
        self._records: dict[str, BatchEnqueuedTokenReservation] = {}
        self.saved = False
        self.refunded: BatchEnqueuedTokenReservation | None = None

    @override
    async def reserve(
        self,
        tokens: int,
        scopes: Sequence[BatchEnqueuedTokenScope],
        litellm_parent_otel_span: Span | None = None,
    ) -> BatchEnqueuedTokenReservation:
        del litellm_parent_otel_span
        self._effects.append("reserve")
        return BatchEnqueuedTokenReservation(
            tokens=tokens,
            scopes=tuple(scopes),
            backend="memory" if self._stages["backend"] is None else self._stages["backend"],
            owner=self._stages["owner"],
        )

    @override
    async def save_reservation(
        self,
        batch_id: str,
        reservation: BatchEnqueuedTokenReservation,
        litellm_parent_otel_span: Span | None = None,
    ) -> None:
        del litellm_parent_otel_span
        self.saved = True
        self._records[batch_id] = reservation

    @override
    async def pop_reservation(
        self, batch_id: str, litellm_parent_otel_span: Span | None = None
    ) -> BatchEnqueuedTokenReservation | None:
        del litellm_parent_otel_span
        return self._records.pop(batch_id, None)

    @override
    async def refund(
        self,
        reservation: BatchEnqueuedTokenReservation,
        litellm_parent_otel_span: Span | None = None,
    ) -> None:
        del litellm_parent_otel_span
        self.refunded = reservation


class ScriptedHook(BatchRateLimiter):
    def __init__(self, stages: StagesJson, effects: list[JsonValue], limiter: Limiter) -> None:
        cache = InternalUsageCache(DualCache())
        super().__init__(internal_usage_cache=cache, parallel_request_limiter=limiter)
        self._stages = stages
        self._effects = effects

    @override
    def _should_skip_batch_input_file_processing(  # ast-grep-ignore: no-long-names  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L338
        self, data: dict[str, JsonValue], user_api_key_dict: UserAPIKeyAuth,
        has_enqueued_scopes: bool = False,
    ) -> tuple[bool, list[RateLimitDescriptor] | None]:
        del data, user_api_key_dict, has_enqueued_scopes
        self._effects.append("should_skip")
        return self._stages["skip"], None

    @override
    async def count_input_file_usage(  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L843
        self,
        file_id: str,
        custom_llm_provider: Literal["openai", "azure", "vertex_ai"] = "openai",
        user_api_key_dict: UserAPIKeyAuth | None = None,
        data: dict[str, JsonValue] | None = None,
        descriptors: Sequence[RateLimitDescriptor] | None = None,
    ) -> BatchFileUsage:
        del file_id, custom_llm_provider, user_api_key_dict, data, descriptors
        self._effects.append("account")
        return BATCH_USAGE.validate_python(self._stages["usage"])


def scripted(value: Input, effects: list[JsonValue]) -> tuple[Limiter, ScriptedStore]:
    store = ScriptedStore(value["stages"], effects)
    limiter = Limiter(internal_usage_cache=InternalUsageCache(DualCache()))
    limiter.batch_enqueued_token_store = store
    raw = value["stash"]["reservation"]
    stash = get_or_create_request_stash()
    stash.batch_enqueued_reservation = None if raw is None else RESERVATION.validate_python(raw)
    return limiter, store


async def lifecycle_answer(value: Input, auth: UserAPIKeyAuth) -> JsonValue:
    call = value["call"]
    effects: list[JsonValue] = []
    limiter, store = scripted(value, effects)
    diagnostics = Sink(auth.api_key)
    if call["op"] == "create":
        hook = ScriptedHook(value["stages"], effects, limiter)
        await hook.async_pre_call_hook(  # type: ignore[misc]  # pyright: ignore[reportAny, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L1180
            user_api_key_dict=auth, cache=DualCache(), data=call["data"],
            call_type=call["call_type"],
        )
        reservation = get_or_create_request_stash().batch_enqueued_reservation
        create_answer: dict[str, JsonValue] = {
            "data": call["data"],
            "diagnostics": diagnostics.lines,
            "effects": effects,
            "reservation": None if reservation is None else reservation_view(reservation),
        }
        return JSON.validate_python(create_answer)
    if call["op"] == "create_failure":
        request_data: dict[str, JsonValue] = {}
        await limiter.async_post_call_failure_hook(  # pyright: ignore[reportAny, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/parallel_request_limiter_v3.py#L5286
            request_data=request_data,
            original_exception=RuntimeError("the batch route failed"),
            user_api_key_dict=UserAPIKeyAuth(),
        )
        refunded = store.refunded
        failure_answer: dict[str, JsonValue] = {
            "refunded": None if refunded is None else reservation_view(refunded),
            "diagnostics": diagnostics.lines,
        }
        return JSON.validate_python(failure_answer)
    data: dict[str, JsonValue] = {}
    response: dict[str, str] = {"id": call["batch_id"], "status": call["status"], "object": "batch"}
    await limiter.async_post_call_success_hook(  # type: ignore[misc]  # pyright: ignore[reportAny, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/parallel_request_limiter_v3.py#L5232
        data=data,
        user_api_key_dict=UserAPIKeyAuth(),
        response=response,  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_logger.py#L491-L497
    )
    success_answer: dict[str, JsonValue] = {
        "batch_id": call["batch_id"],
        "saved": store.saved,
        "refunded": store.refunded is not None,
        "diagnostics": diagnostics.lines,
    }
    return JSON.validate_python(success_answer)


async def retrieve_answer(value: Input, auth: UserAPIKeyAuth) -> JsonValue:
    call = value["call"]
    effects: list[JsonValue] = []
    memory = InMemoryCache()
    memory.cache_dict = dict(cast(dict[str, JsonValue], value["memory"]["entries"]))  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/in_memory_cache.py#L158
    remote = ScriptedRedis(value["remote"], effects)
    litellm.cache = Cache(_backend=remote, type=LiteLLMCacheType.REDIS)
    diagnostics = Sink(auth.api_key)
    hook = BatchRedisRequests()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_redis_get.py#L18
    await hook.async_pre_call_hook(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_redis_get.py#L34
        user_api_key_dict=auth,
        cache=DualCache(in_memory_cache=memory),
        data=call["data"],
        call_type=call["call_type"],
    )
    cached = cast(JsonValue, await hook.async_get_cache(cache_key=call["cache_key"]))  # type: ignore[no-untyped-call, misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_redis_get.py#L91
    retrieve_answer_json: dict[str, JsonValue] = {
        "data": call["data"],
        "cached": cached,
        "diagnostics": diagnostics.lines,
        "effects": effects,
    }
    return JSON.validate_python(retrieve_answer_json)


def run(input_value: Input) -> JsonValue:
    with contextlib.redirect_stdout(io.StringIO()):
        call = input_value["call"]
        if call["op"] in ("create", "retrieve"):
            auth = TypeAdapter(UserAPIKeyAuth).validate_python(call["principal"])
            if call["op"] == "create":
                return asyncio.run(lifecycle_answer(input_value, auth))
            return asyncio.run(retrieve_answer(input_value, auth))
        return asyncio.run(lifecycle_answer(input_value, UserAPIKeyAuth()))
