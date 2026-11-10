import asyncio
from datetime import UTC, datetime
from typing import Final, cast
from unittest.mock import patch

from litellm.caching.dual_cache import DualCache
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.exceptions import RateLimitError
from litellm.router_utils.pre_call_checks import io_token_rate_limit_check as io_check
from litellm.router_utils.pre_call_checks.io_token_rate_limit_check import (
    _get_cache_keys,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]
    _read_reservation_from_kwargs,  # pyright: ignore[reportPrivateUsage]
    _reservation_channels,  # pyright: ignore[reportPrivateUsage]
)
from pydantic import Field, JsonValue
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

ROUTING_ARGS_TTL: Final = 60
MILLIS_PER_SECOND: Final = 1000


class DeploymentInput(Strict):
    id: str | None = None
    model: str | None = None
    itpm: int | None = None
    otpm: int | None = None


class RequestInput(Strict):
    messages: JsonValue | None = None
    prompt: JsonValue | None = None
    input: JsonValue | None = None
    max_tokens: JsonValue | None = None
    max_completion_tokens: JsonValue | None = None
    max_output_tokens: JsonValue | None = None


class CounterInput(Strict):
    itpm: int = 0
    otpm: int = 0


class Reservation(Strict):
    itpm_reserved: int
    otpm_reserved: int
    itpm_key: str | None
    otpm_key: str | None


class CounterValues(Strict):
    itpm: int
    otpm: int


class ErrorFields(Strict):
    status_code: int
    code: str
    error_type: str
    retry_after_seconds: int


class Output(Strict):
    reservation: Reservation | None
    counters: CounterValues
    error: ErrorFields | None
    metadata: dict[str, JsonValue] | None
    litellm_metadata: dict[str, JsonValue] | None


class IoTokenReservations(Operation):
    now_millis: int
    deployment: DeploymentInput
    request: RequestInput
    estimate: int
    counters: CounterInput = Field(default_factory=CounterInput)
    litellm_metadata: dict[str, JsonValue] | None = None

    @override
    def run(self) -> Strict:
        now = datetime.fromtimestamp(self.now_millis / MILLIS_PER_SECOND, UTC)
        kwargs = request_kwargs(self.request)
        if self.litellm_metadata is not None:
            kwargs["litellm_metadata"] = dict(self.litellm_metadata)
        io_check.set_io_token_rate_limit_request_kwargs(kwargs)
        cache = DualCache(in_memory_cache=InMemoryCache())
        deployment = deployment_dict(self.deployment)
        keys = _get_cache_keys(deployment, now.strftime("%H-%M"))
        if keys is None:
            itpm_key: str | None = None
            otpm_key: str | None = None
        else:
            itpm_key, otpm_key = keys
            seed(cache, itpm_key, self.counters.itpm)
            seed(cache, otpm_key, self.counters.otpm)
        error: ErrorFields | None = None

        def fixed_estimate(_request_kwargs: dict[str, object], **_kwargs: object) -> int:
            return self.estimate

        with (
            patch.object(io_check, "get_utc_datetime", return_value=now),
            patch.object(io_check, "_estimate_input_tokens", fixed_estimate),
        ):
            try:
                _result = asyncio.run(  # pyright: ignore[reportUnknownVariableType]
                    io_check.async_io_token_pre_call_check(  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]
                        cache, deployment
                    )
                )
            except RateLimitError as raised:
                error = error_fields(raised)
        return Output(
            reservation=reservation(kwargs),
            counters=CounterValues(
                itpm=counter_value(cache, itpm_key),
                otpm=counter_value(cache, otpm_key),
            ),
            error=error,
            metadata=object_channel(kwargs.get("metadata")),
            litellm_metadata=object_channel(kwargs.get("litellm_metadata")),
        )


def request_kwargs(request: RequestInput) -> dict[str, object]:
    kwargs: dict[str, object] = {}
    if request.messages is not None:
        kwargs["messages"] = request.messages
    if request.prompt is not None:
        kwargs["prompt"] = request.prompt
    if request.input is not None:
        kwargs["input"] = request.input
    if request.max_tokens is not None:
        kwargs["max_tokens"] = request.max_tokens
    if request.max_completion_tokens is not None:
        kwargs["max_completion_tokens"] = request.max_completion_tokens
    if request.max_output_tokens is not None:
        kwargs["max_output_tokens"] = request.max_output_tokens
    return kwargs


def deployment_dict(deployment: DeploymentInput) -> dict[str, object]:
    model_info: dict[str, object] = {}
    params: dict[str, object] = {}
    if deployment.id is not None:
        model_info["id"] = deployment.id
    if deployment.model is not None:
        params["model"] = deployment.model
    payload: dict[str, object] = {"model_info": model_info, "litellm_params": params}
    if deployment.itpm is not None:
        payload["itpm"] = deployment.itpm
    if deployment.otpm is not None:
        payload["otpm"] = deployment.otpm
    return payload


def seed(cache: DualCache, key: str, value: int) -> None:
    cache.in_memory_cache.set_cache(key=key, value=value, ttl=ROUTING_ARGS_TTL)  # pyright: ignore[reportUnknownMemberType]


def counter_value(cache: DualCache, key: str | None) -> int:
    if key is None:
        return 0
    stored = cast("int | float | None", cache.in_memory_cache.get_cache(key=key))  # pyright: ignore[reportUnknownMemberType]
    return int(stored or 0)


def reservation(kwargs: dict[str, object]) -> Reservation | None:
    for channel in _reservation_channels(kwargs):
        if isinstance(channel, dict) and io_check.ITPM_RESERVED_KEY in channel:
            itpm_reserved, otpm_reserved, itpm_key, otpm_key = _read_reservation_from_kwargs(kwargs)
            return Reservation(
                itpm_reserved=itpm_reserved,
                otpm_reserved=otpm_reserved,
                itpm_key=itpm_key,
                otpm_key=otpm_key,
            )
    return None


def object_channel(value: object) -> dict[str, JsonValue] | None:
    if not isinstance(value, dict):
        return None
    return cast("dict[str, JsonValue]", value)


def error_fields(error: RateLimitError) -> ErrorFields:
    return ErrorFields(
        status_code=error.status_code,
        code=cast("str", error.code),
        error_type=cast("str", error.type),
        retry_after_seconds=int(error.response.headers["retry-after"]),
    )
