from typing import Literal, Protocol, runtime_checkable

from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.hooks import PROXY_HOOKS
from litellm.proxy.hooks.batch_enqueued_tokens import BatchEnqueuedTokenReservation
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    ParallelSlotAcquisition,
    RateLimitResponse,
    RequestRateLimiterStash,
    get_or_create_request_stash,
)
from litellm.types.utils import EmbeddingResponse
from pydantic import ConfigDict, TypeAdapter

from .fakes import FakeBatchStore, GroupMap, HiddenResponse, RecordingCache
from .inputs import CallbackArguments, ResponseInput, StashInput, WindowInput

ReservationWindow = tuple[str, str, Literal["redis", "local"]]


class Limiter(Protocol):
    batch_enqueued_token_store: FakeBatchStore

    async def async_log_success_event(
        self,
        kwargs: CallbackArguments,
        response_obj: EmbeddingResponse,
        start_time: float,
        end_time: float,
    ) -> None: ...

    async def async_log_failure_event(
        self,
        kwargs: CallbackArguments,
        response_obj: EmbeddingResponse | None,
        start_time: float,
        end_time: float,
    ) -> None: ...

    async def async_logging_hook(
        self,
        kwargs: CallbackArguments,
        result: HiddenResponse | None,
        call_type: str,
    ) -> tuple[CallbackArguments, HiddenResponse | None]: ...

    async def async_post_call_success_hook(
        self,
        data: CallbackArguments,
        user_api_key_dict: UserAPIKeyAuth,
        response: ResponseInput,
    ) -> None: ...

    async def async_post_call_failure_hook(
        self,
        request_data: CallbackArguments,
        original_exception: Exception,
        user_api_key_dict: UserAPIKeyAuth,
        traceback_str: str | None = None,
    ) -> None: ...

    async def async_release_max_parallel_requests_on_disconnect(  # ast-grep-ignore: no-long-names
        self, user_api_key_dict: UserAPIKeyAuth
    ) -> None: ...


@runtime_checkable
class LimiterConstructor(Protocol):
    def __call__(
        self, cache: RecordingCache, *, model_group_resolver: GroupMap
    ) -> Limiter: ...


def make_limiter(cache: RecordingCache) -> Limiter:
    adapter = TypeAdapter[LimiterConstructor](
        LimiterConstructor, config=ConfigDict(arbitrary_types_allowed=True)
    )
    constructor = adapter.validate_python(PROXY_HOOKS["parallel_request_limiter"])
    return constructor(cache, model_group_resolver=GroupMap())


def scope_set(pairs: list[list[str]]) -> frozenset[tuple[str, str]]:
    scopes = {(pair[0], pair[1]) for pair in pairs}
    return frozenset(scopes)


def window_set(windows: list[WindowInput]) -> frozenset[ReservationWindow]:
    identities: set[ReservationWindow] = set()
    for window in windows:
        identities.add((window["counter_key"], window["window_start"], window["backend"]))
    return frozenset(identities)


def install_slot_fields(stash: RequestRateLimiterStash, input_value: StashInput) -> None:
    slot = input_value.get("parallel_slot")
    if slot is not None:
        stash.parallel_slot = ParallelSlotAcquisition(
            slot_id=slot["slot_id"], counter_keys=slot["counter_keys"]
        )
    statuses = input_value.get("statuses")
    if statuses is not None:
        stash.rate_limit_response = RateLimitResponse(overall_code="OK", statuses=statuses)


def install_reservation_fields(stash: RequestRateLimiterStash, input_value: StashInput) -> None:
    reserved = input_value.get("reserved_tokens")
    if reserved is not None:
        stash.reserved_tokens = reserved
    scopes = input_value.get("reserved_scopes")
    if scopes is not None:
        stash.reserved_scopes = scope_set(scopes)
    itpm_reserved = input_value.get("itpm_reserved_tokens")
    if itpm_reserved is not None:
        stash.itpm_reserved_tokens = itpm_reserved
    itpm_scopes = input_value.get("itpm_reserved_scopes")
    if itpm_scopes is not None:
        stash.itpm_reserved_scopes = scope_set(itpm_scopes)
    itpm_windows = input_value.get("itpm_reserved_windows")
    if itpm_windows is not None:
        stash.itpm_reserved_window_identities = window_set(itpm_windows)
    otpm_reserved = input_value.get("otpm_reserved_tokens")
    if otpm_reserved is not None:
        stash.otpm_reserved_tokens = otpm_reserved
    otpm_scopes = input_value.get("otpm_reserved_scopes")
    if otpm_scopes is not None:
        stash.otpm_reserved_scopes = scope_set(otpm_scopes)
    otpm_windows = input_value.get("otpm_reserved_windows")
    if otpm_windows is not None:
        stash.otpm_reserved_window_identities = window_set(otpm_windows)


def install_batch_fields(stash: RequestRateLimiterStash, input_value: StashInput) -> None:
    owner = input_value.get("owner_call_id")
    if owner is not None:
        stash.owner_litellm_call_id = owner
    batch_tokens = input_value.get("batch_tokens")
    if batch_tokens is not None:
        stash.batch_enqueued_reservation = BatchEnqueuedTokenReservation(
            tokens=batch_tokens,
            scopes=(),
            backend="redis",
            owner="worker",
            reserved_at_monotonic=0.0,
        )
    tpd_ops = input_value.get("batch_tpd_ops")
    if tpd_ops is not None:
        stash.batch_tpd_refund_ops = tuple(tpd_ops)
    released = input_value.get("reservation_released")
    if released is not None:
        stash.reservation_released = released


def install_stash(input_value: StashInput | None) -> RequestRateLimiterStash:
    stash = get_or_create_request_stash()
    if input_value is None:
        return stash
    install_slot_fields(stash, input_value)
    install_reservation_fields(stash, input_value)
    install_batch_fields(stash, input_value)
    return stash
