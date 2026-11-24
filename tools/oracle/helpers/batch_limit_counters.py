import asyncio
from collections.abc import Callable, Mapping
from typing import TypeAlias, TypedDict, cast

from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.common_utils.proxy_rate_limit_error import ProxyRateLimitError
from litellm.proxy.hooks.batch_enqueued_tokens import (
    BatchEnqueuedTokenOverLimit,
    BatchEnqueuedTokenScope,
    BatchEnqueuedTokenStore,
)
from litellm.proxy.hooks.batch_rate_limiter import BatchFileUsage
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    RateLimitDescriptor,
    RateLimitStatus,
    get_or_create_request_stash,
)
from litellm.proxy.utils import InternalUsageCache
from pydantic import JsonValue, TypeAdapter

from batch_limit_counters_fakes import (
    ScriptedCache,
    ScriptedHook,
    ScriptedLimiter,
    ScriptedStore,
    Trace,
)
from batch_limit_counters_records import (
    BATCH_USAGE,
    JSON,
    Call,
    Effects,
    ErrorView,
    Function,
    LimiterScript,
    OperationView,
    StoreScript,
    error_view,
    operation_view,
    reservation_view,
    response_of,
    status_of,
    windows_of,
)


class Input(TypedDict):
    principal: dict[str, JsonValue]
    clock: float
    window_size: int
    cache: dict[str, JsonValue]
    store: StoreScript
    limiter: LimiterScript
    calls: list[Call]


class RejectionAnswer(TypedDict):
    function: Function
    rejection: ErrorView | None


class ErrorAnswer(TypedDict):
    function: Function
    error: ErrorView


class WindowAnswer(TypedDict):
    function: Function
    window_start: int | None


class RefundAnswer(TypedDict):
    function: Function
    operations: list[OperationView]


Answer: TypeAlias = RejectionAnswer | ErrorAnswer | WindowAnswer | RefundAnswer


EnqueuedRejectionRaiser: TypeAlias = Callable[
    [BatchEnqueuedTokenOverLimit, Mapping[str, JsonValue], BatchFileUsage], None
]
RateRejectionRaiser: TypeAlias = Callable[
    [RateLimitStatus, list[RateLimitDescriptor], BatchFileUsage, str, str | None, int | None], None
]


def rejection_answer(call: Call, error: ProxyRateLimitError) -> RejectionAnswer | ErrorAnswer:
    if call["function"] in ("raise_enqueued_limit_error", "raise_rate_limit_error"):
        return {"function": call["function"], "error": error_view(error)}
    return {"function": call["function"], "rejection": error_view(error)}


async def invoke(call: Call, hook: ScriptedHook, auth: UserAPIKeyAuth) -> Answer:
    function = call["function"]
    if function == "reserve_batch_enqueued_tokens":
        await hook._reserve_batch_enqueued_tokens(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L565
            user_api_key_dict=auth,
            data=call["data"],
            batch_usage=BATCH_USAGE.validate_python(call["batch_usage"]),
            scopes=tuple(
                BatchEnqueuedTokenScope(
                    key=scope["key"], value=scope["value"], limit=scope["limit"]
                )
                for scope in call["scopes"]
            ),
        )
        return {"function": function, "rejection": None}
    if function == "raise_enqueued_limit_error":
        over_limit = call["over_limit"]
        raise_enqueued: EnqueuedRejectionRaiser = hook._raise_enqueued_limit_error  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L591
        raise_enqueued(
            BatchEnqueuedTokenOverLimit(
                scope=BatchEnqueuedTokenScope(
                    key=over_limit["scope"]["key"],
                    value=over_limit["scope"]["value"],
                    limit=over_limit["scope"]["limit"],
                ),
                enqueued=over_limit["enqueued"],
            ),
            call["data"],
            BATCH_USAGE.validate_python(call["batch_usage"]),
        )
    elif function == "raise_rate_limit_error":
        raise_rate: RateRejectionRaiser = hook._raise_rate_limit_error  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L618
        raise_rate(
            status_of(call["status"]),
            call["descriptors"],
            BATCH_USAGE.validate_python(call["batch_usage"]),
            call["limit_type"],
            call.get("requested_model"),
            call.get("window_start"),
        )
    elif function == "check_and_increment_batch_counters":
        await hook._check_and_increment_batch_counters(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L719
            user_api_key_dict=auth,
            data=call["data"],
            batch_usage=BATCH_USAGE.validate_python(call["batch_usage"]),
            descriptors=call.get("descriptors"),
        )
        return {"function": function, "rejection": None}
    elif function == "read_tpd_window_start":
        window_start = await hook._read_tpd_window_start(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L790
            status=status_of(call["status"]),
            parent_otel_span=None,
        )
        return {"function": function, "window_start": window_start}
    else:
        operations = hook._build_tpd_refund_ops(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L806
            call["descriptors"],
            call["tokens"],
            windows_of(call.get("reservation_windows")),
        )
        return {"function": function, "operations": [operation_view(op) for op in operations]}
    raise AssertionError("a batch rejection callable returned instead of raising")


async def one(call: Call, hook: ScriptedHook, auth: UserAPIKeyAuth) -> tuple[Answer, bool]:
    try:
        return await invoke(call, hook, auth), False
    except ProxyRateLimitError as raised:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/proxy_rate_limit_error.py#L101
        return rejection_answer(call, raised), True


async def drive(
    calls: list[Call], hook: ScriptedHook, auth: UserAPIKeyAuth, effects: Effects
) -> list[Answer]:
    answers: list[Answer] = []
    for call in calls:
        answer, raised = await one(call, hook, auth)
        answers.append(answer)
        if raised:
            break
    stash = get_or_create_request_stash()
    reservation = stash.batch_enqueued_reservation
    effects["reservation"] = None if reservation is None else reservation_view(reservation)
    effects["refund_ops"] = [operation_view(op) for op in stash.batch_tpd_refund_ops]
    return answers


def run(input_value: Input) -> JsonValue:
    auth = TypeAdapter(UserAPIKeyAuth).validate_python(input_value["principal"])
    trace = Trace()
    cache = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L378
        InternalUsageCache, ScriptedCache(trace, input_value["cache"])
    )
    store = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_enqueued_tokens.py#L177
        BatchEnqueuedTokenStore, ScriptedStore(trace, input_value["store"]["replies"])
    )
    script = input_value["limiter"]
    responses = [response_of(raw) for raw in script["responses"]]
    limiter = ScriptedLimiter(cache, store, trace, responses, input_value["window_size"])
    hook = ScriptedHook(cache, limiter, input_value["clock"], script, trace)
    effects: Effects = {
        "store_calls": trace.store_calls,
        "limiter_calls": trace.limiter_calls,
        "base_calls": 0,
        "project_calls": 0,
        "cache_reads": trace.cache_reads,
        "reservation": None,
        "refund_ops": [],
    }
    answers = asyncio.run(drive(input_value["calls"], hook, auth, effects))
    effects["base_calls"] = trace.base_calls
    effects["project_calls"] = trace.project_calls
    return JSON.validate_python({"answers": answers, "effects": effects})  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/batch_rate_limiter.py#L719
