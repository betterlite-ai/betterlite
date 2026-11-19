import asyncio
from typing import NamedTuple, TypedDict

from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.hooks.batch_enqueued_tokens import BatchEnqueuedTokenReservation
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    ParallelSlotAcquisition,
    RequestRateLimiterStash,
)
from litellm.types.utils import EmbeddingResponse, Usage
from pydantic import JsonValue, TypeAdapter

from .inputs import (
    CallbackArguments,
    Case,
    HiddenParams,
    Input,
    LiteLLMParams,
    LoggingPayload,
    ResponseInput,
)
from .operations import (
    FakeBatchStore,
    HiddenResponse,
    Limiter,
    RecordingCache,
    install_stash,
    make_limiter,
)


class Record(TypedDict):
    slp_headers: dict[str, JsonValue] | None
    response_headers: dict[str, JsonValue] | None
    parallel_slot: ParallelSlotAcquisition | None
    reservation_released: bool
    batch_tokens: int | None
    tpd_ops: int
    gauge_reads: list[JsonValue]
    gauge_writes: list[JsonValue]
    increments: list[JsonValue]
    pipelines: list[JsonValue]
    local_sets: list[JsonValue]
    batch_calls: list[JsonValue]


class Fixture(NamedTuple):
    limiter: Limiter
    stash: RequestRateLimiterStash
    cache: RecordingCache
    store: FakeBatchStore


def metadata_of(case: Case) -> dict[str, JsonValue]:
    logging = case.get("logging")
    if logging is None:
        return {}
    return logging


def usage_of(case: Case) -> Usage | None:
    usage = case.get("usage")
    if usage is None:
        return None
    return TypeAdapter(Usage).validate_python(usage)


def callback_arguments(case: Case) -> CallbackArguments:
    arguments = CallbackArguments()
    call_id = case.get("call_id")
    if call_id is not None:
        arguments["litellm_call_id"] = call_id
    metadata = metadata_of(case)
    arguments["litellm_params"] = LiteLLMParams(metadata=metadata)
    arguments["standard_logging_object"] = LoggingPayload(metadata=metadata)
    usage = usage_of(case)
    if usage is not None:
        arguments["combined_usage_object"] = usage
    return arguments


def logging_arguments(case: Case, headers: dict[str, JsonValue]) -> CallbackArguments:
    payload: LoggingPayload = {"hidden_params": HiddenParams(additional_headers=headers)}
    arguments = CallbackArguments(standard_logging_object=payload)
    call_id = case.get("call_id")
    if call_id is not None:
        arguments["litellm_call_id"] = call_id
    return arguments


def logging_headers(arguments: CallbackArguments) -> dict[str, JsonValue] | None:
    payload = arguments.get("standard_logging_object")
    if payload is None:
        return None
    hidden = payload.get("hidden_params")
    if hidden is None:
        return None
    return hidden["additional_headers"]


def embedding_response(case: Case) -> EmbeddingResponse:
    usage = usage_of(case)
    if usage is None:
        return EmbeddingResponse()
    return EmbeddingResponse(usage=usage)


def popped_reservation(tokens: int | None) -> BatchEnqueuedTokenReservation | None:
    if tokens is None:
        return None
    return BatchEnqueuedTokenReservation(
        tokens=tokens,
        scopes=(),
        backend="redis",
        owner="worker",
        reserved_at_monotonic=0.0,
    )


def make_fixture(case: Case) -> Fixture:
    stash_input = case.get("stash")
    stash = install_stash(stash_input)
    gauge_keys: set[str] = set()
    slot = stash.parallel_slot
    if slot is not None:
        gauge_keys = set(slot["counter_keys"])
    seed = case.get("seed")
    values: dict[str, JsonValue] = {}
    if seed is not None:
        values = seed
    cache = RecordingCache(values, gauge_keys)
    store = FakeBatchStore(popped_reservation(case.get("pop_tokens")))
    limiter = make_limiter(cache)
    limiter.batch_enqueued_token_store = store
    return Fixture(limiter=limiter, stash=stash, cache=cache, store=store)


def record(
    fixture: Fixture,
    slp_headers: dict[str, JsonValue] | None,
    response_headers: dict[str, JsonValue] | None,
) -> Record:
    stash = fixture.stash
    reservation: BatchEnqueuedTokenReservation | None = stash.batch_enqueued_reservation
    tokens: int | None = None
    if reservation is not None:
        tokens = reservation.tokens
    return Record(
        slp_headers=slp_headers,
        response_headers=response_headers,
        parallel_slot=stash.parallel_slot,
        reservation_released=stash.reservation_released,
        batch_tokens=tokens,
        tpd_ops=len(stash.batch_tpd_refund_ops),
        gauge_reads=fixture.cache.gauge_reads,
        gauge_writes=fixture.cache.gauge_writes,
        increments=fixture.cache.increments,
        pipelines=fixture.cache.pipelines,
        local_sets=fixture.cache.local_sets,
        batch_calls=fixture.store.calls,
    )


async def logging_answer(case: Case, fixture: Fixture, *, mirror: bool) -> Record:
    headers = case.get("headers")
    slp_headers: dict[str, JsonValue] = {}
    if headers is not None:
        slp_headers = headers
    arguments = logging_arguments(case, slp_headers)
    result = HiddenResponse({}) if mirror else None
    runs = case.get("runs")
    for _ in range(1 if runs is None else runs):
        await fixture.limiter.async_logging_hook(arguments, result, "completion")
    response_headers: dict[str, JsonValue] | None = None
    if result is not None:
        response_headers = result.additional_headers()
    return record(fixture, logging_headers(arguments), response_headers)


async def success_event_answer(case: Case, fixture: Fixture) -> Record:
    arguments = callback_arguments(case)
    await fixture.limiter.async_log_success_event(arguments, embedding_response(case), 0.0, 0.0)
    return record(fixture, None, None)


async def failure_event_answer(case: Case, fixture: Fixture) -> Record:
    arguments = callback_arguments(case)
    await fixture.limiter.async_log_failure_event(arguments, None, 0.0, 0.0)
    return record(fixture, None, None)


async def disconnect_answer(case: Case, fixture: Fixture) -> Record:
    await fixture.limiter.async_release_max_parallel_requests_on_disconnect(UserAPIKeyAuth())
    return record(fixture, None, None)


async def success_hook_answer(case: Case, fixture: Fixture) -> Record:
    arguments = CallbackArguments()
    call_id = case.get("call_id")
    if call_id is not None:
        arguments["litellm_call_id"] = call_id
    given = case.get("response")
    response = ResponseInput() if given is None else given
    await fixture.limiter.async_post_call_success_hook(arguments, UserAPIKeyAuth(), response)
    hidden = response.get("_hidden_params")
    headers: dict[str, JsonValue] | None = None
    if hidden is not None:
        headers = hidden["additional_headers"]
    return record(fixture, None, headers)


async def failure_hook_answer(case: Case, fixture: Fixture) -> Record:
    arguments = callback_arguments(case)
    error = RuntimeError("the proxy hook rejected the request")
    await fixture.limiter.async_post_call_failure_hook(arguments, error, UserAPIKeyAuth())
    return record(fixture, None, None)


async def answer(input_value: Case, fixture: Fixture) -> Record:
    function = input_value["function"]
    if function == "header_merge":
        return await logging_answer(input_value, fixture, mirror=False)
    if function == "logging_hook":
        return await logging_answer(input_value, fixture, mirror=True)
    if function == "success_event":
        return await success_event_answer(input_value, fixture)
    if function == "failure_event":
        return await failure_event_answer(input_value, fixture)
    if function == "disconnect":
        return await disconnect_answer(input_value, fixture)
    if function == "success_hook":
        return await success_hook_answer(input_value, fixture)
    return await failure_hook_answer(input_value, fixture)


def run(input_value: Input) -> Record:
    fixture = make_fixture(input_value)
    with asyncio.Runner() as runner:
        return runner.run(answer(input_value, fixture))
