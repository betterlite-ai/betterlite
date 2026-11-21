import asyncio
from collections.abc import Sequence
from typing import Protocol, TypedDict, runtime_checkable

import litellm
from b3042.answers import raised_json, response_json, window_json
from b3042.fakes import FixedFailClosed, FixedTime, GroupMap, RecordingCache
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.hooks import PROXY_HOOKS
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    ParallelSlotAcquisition,
    RateLimitDescriptor,
    RateLimitResponse,
    RequestRateLimiterStash,
    get_or_create_request_stash,
)
from pydantic import ConfigDict, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

__all__ = ["run"]

class Input(TypedDict):
    descriptors: list[RateLimitDescriptor]
    data: JsonValue
    requested_model: str | None
    call_type: str | None
    tpm_reservation_scopes: list[list[str]]
    tpm_reservation_amount: int
    now: int
    cache: dict[str, int | float | str]
    parallel_slot: ParallelSlotAcquisition | None
    rate_limit_response: RateLimitResponse | None


class ProjectAdmission(Protocol):
    async def _reserve_project_io_tokens_or_raise(
        self,
        descriptors: Sequence[RateLimitDescriptor],
        data: JsonValue,
        requested_model: str | None,
        user_api_key_dict: UserAPIKeyAuth,
        tpm_reservation_scopes: Sequence[tuple[str, str]],
        tpm_reservation_amount: int,
        call_type: str | None,
    ) -> None: ...

    async def admit(self, call: Input) -> None:
        scopes = [(scope[0], scope[1]) for scope in call["tpm_reservation_scopes"]]
        await self._reserve_project_io_tokens_or_raise(
            call["descriptors"],
            call["data"],
            call["requested_model"],
            UserAPIKeyAuth(),
            scopes,
            call["tpm_reservation_amount"],
            call["call_type"],
        )


@runtime_checkable
class LimiterClass(Protocol):
    def __call__(
        self,
        internal_usage_cache: RecordingCache,
        *,
        time_provider: FixedTime,
        model_group_resolver: GroupMap,
        fail_closed_resolver: FixedFailClosed,
    ) -> ProjectAdmission: ...


def stash_json(stash: RequestRateLimiterStash) -> JsonValue:
    stored = stash.rate_limit_response
    rendered: dict[str, JsonValue] = {
        "owner_litellm_call_id": stash.owner_litellm_call_id,
        "reservation_released": stash.reservation_released,
        "parallel_slot_released": stash.parallel_slot is None,
        "itpm_reserved_tokens": stash.itpm_reserved_tokens,
        "itpm_reserved_scopes": [list(scope) for scope in sorted(stash.itpm_reserved_scopes)],
        "itpm_reserved_windows": [
            window_json(key, start, backend)
            for key, start, backend in sorted(stash.itpm_reserved_window_identities)
        ],
        "otpm_reserved_tokens": stash.otpm_reserved_tokens,
        "otpm_reserved_scopes": [list(scope) for scope in sorted(stash.otpm_reserved_scopes)],
        "otpm_reserved_windows": [
            window_json(key, start, backend)
            for key, start, backend in sorted(stash.otpm_reserved_window_identities)
        ],
        "rate_limit_response": None if stored is None else response_json(stored),
    }
    return rendered


async def handle(call: Input) -> JsonValue:
    cache = RecordingCache(call["cache"])
    limiter_class = TypeAdapter[LimiterClass](
        LimiterClass, config=ConfigDict(arbitrary_types_allowed=True)
    ).validate_python(PROXY_HOOKS["parallel_request_limiter"])
    limiter = limiter_class(
        cache,
        time_provider=FixedTime(call["now"]),
        model_group_resolver=GroupMap({}),
        fail_closed_resolver=FixedFailClosed(enabled=False),
    )
    stash = get_or_create_request_stash()
    stash.parallel_slot = call["parallel_slot"]
    stash.rate_limit_response = call["rate_limit_response"]
    task: asyncio.Future[None] = asyncio.ensure_future(ProjectAdmission.admit(limiter, call))
    _done, _pending = await asyncio.wait({task})
    error = task.exception()
    rendered: dict[str, JsonValue] = {
        "raised": None if error is None else raised_json(error),
        "data": call["data"],
        "stash": stash_json(stash),
        "effects": {
            "reads": cache.reads,
            "writes": cache.writes,
            "increments": cache.increments,
            "pipelines": cache.pipelines,
        },
    }
    return rendered


def run(input_value: Input) -> JsonValue:
    with asyncio.Runner() as runner:
        return runner.run(handle(input_value))
