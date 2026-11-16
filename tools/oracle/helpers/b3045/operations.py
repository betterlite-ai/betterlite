from collections.abc import Mapping, Sequence
from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict

from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    RateLimitedModel,
    RequestRateLimiterStash,
    get_or_create_request_stash,
)
from pydantic import JsonValue

from .inputs import CallbackArguments, ResponsePayload, StashInput, WindowInput

ReservationWindow: TypeAlias = tuple[str, str, Literal["redis", "local"]]


# short mirror of LiteLLM's RedisPipelineIncrementOperation and ReservationAwareIncrementOperation
class PipelineOperation(TypedDict):
    key: str
    increment_value: float
    ttl: int | None


class ReservationOperation(PipelineOperation):
    window_key: NotRequired[str]
    expected_window_start: NotRequired[str]
    reservation_backend: NotRequired[Literal["redis", "local"]]


class ReservationJson(TypedDict):
    key: str
    increment_value: float
    ttl: int | None
    window_key: str | None
    expected_window_start: str | None
    reservation_backend: str | None


class ReservationAnswer(TypedDict):
    operations: list[ReservationJson]


class SuccessAnswer(TypedDict):
    operations: list[PipelineOperation]


class TargetsAnswer(TypedDict):
    targets: list[list[str]]


Answer: TypeAlias = ReservationAnswer | SuccessAnswer | TargetsAnswer


def reservation_operation(operation: ReservationOperation) -> ReservationJson:
    return {
        "key": operation["key"],
        "increment_value": float(operation["increment_value"]),
        "ttl": operation["ttl"],
        "window_key": operation.get("window_key"),
        "expected_window_start": operation.get("expected_window_start"),
        "reservation_backend": operation.get("reservation_backend"),
    }


def ordered_reservations(
    operations: Sequence[ReservationOperation],
) -> list[ReservationJson]:
    converted = {
        operation["key"]: reservation_operation(operation) for operation in operations
    }
    return [converted[key] for key in sorted(converted)]


def success_operation(operation: PipelineOperation) -> PipelineOperation:
    return {
        "key": operation["key"],
        "increment_value": float(operation["increment_value"]),
        "ttl": operation["ttl"],
    }


class Operations(Protocol):
    def _collect_tpm_scope_targets(
        self,
        standard_logging_metadata: Mapping[str, JsonValue],
        kwargs: CallbackArguments,
        model_group: str | None,
        tpm_limited_tags: set[str],
    ) -> list[tuple[str, str]]: ...

    def _build_io_token_reservation_ops(  # ast-grep-ignore: no-long-names
        self, kwargs: CallbackArguments, response_obj: ResponsePayload
    ) -> Sequence[ReservationOperation]: ...

    def _build_success_event_pipeline_operations(  # ast-grep-ignore: no-long-names
        self,
        kwargs: CallbackArguments,
        response_obj: ResponsePayload,
        rate_limit_type: Literal["input", "output", "total"],
    ) -> list[PipelineOperation]: ...

    def io_token_ops(
        self, kwargs: CallbackArguments, response_obj: ResponsePayload
    ) -> ReservationAnswer:
        operations = self._build_io_token_reservation_ops(kwargs, response_obj)
        return ReservationAnswer(operations=ordered_reservations(operations))

    def scope_targets(
        self,
        standard_logging_metadata: Mapping[str, JsonValue],
        kwargs: CallbackArguments,
        model_group: str | None,
        tags: set[str],
    ) -> TargetsAnswer:
        targets = self._collect_tpm_scope_targets(
            standard_logging_metadata=standard_logging_metadata,
            kwargs=kwargs,
            model_group=model_group,
            tpm_limited_tags=tags,
        )
        return TargetsAnswer(targets=[[key, value] for key, value in targets])

    def success_ops(
        self,
        kwargs: CallbackArguments,
        response_obj: ResponsePayload,
        rate_limit_type: Literal["input", "output", "total"],
    ) -> SuccessAnswer:
        operations = self._build_success_event_pipeline_operations(
            kwargs, response_obj, rate_limit_type
        )
        return SuccessAnswer(
            operations=[success_operation(operation) for operation in operations]
        )


def window_set(windows: list[WindowInput]) -> frozenset[ReservationWindow]:
    identities: set[ReservationWindow] = set()
    for window in windows:
        identities.add((window["counter_key"], window["window_start"], window["backend"]))
    return frozenset(identities)


def install_stash_scopes(stash: RequestRateLimiterStash, input_value: StashInput) -> None:
    scopes = input_value.get("reserved_scopes")
    if scopes is not None:
        stash.reserved_scopes = frozenset((pair[0], pair[1]) for pair in scopes)
    input_reserved = input_value.get("itpm_reserved_tokens")
    if input_reserved is not None:
        stash.itpm_reserved_tokens = input_reserved
    input_scopes = input_value.get("itpm_reserved_scopes")
    if input_scopes is not None:
        stash.itpm_reserved_scopes = frozenset((pair[0], pair[1]) for pair in input_scopes)
    input_windows = input_value.get("itpm_reserved_windows")
    if input_windows is not None:
        stash.itpm_reserved_window_identities = window_set(input_windows)
    output_reserved = input_value.get("otpm_reserved_tokens")
    if output_reserved is not None:
        stash.otpm_reserved_tokens = output_reserved
    output_scopes = input_value.get("otpm_reserved_scopes")
    if output_scopes is not None:
        stash.otpm_reserved_scopes = frozenset((pair[0], pair[1]) for pair in output_scopes)
    output_windows = input_value.get("otpm_reserved_windows")
    if output_windows is not None:
        stash.otpm_reserved_window_identities = window_set(output_windows)


def install_stash(input_value: StashInput) -> None:
    stash = get_or_create_request_stash()
    owner = input_value.get("owner_call_id")
    if owner is not None:
        stash.owner_litellm_call_id = owner
    reserved = input_value.get("reserved_tokens")
    if reserved is not None:
        stash.reserved_tokens = reserved
    model = input_value.get("reserved_model")
    if model is not None:
        stash.reserved_model = RateLimitedModel(
            requested=model["requested"], group=model["group"]
        )
    install_stash_scopes(stash, input_value)
    released = input_value.get("reservation_released")
    if released is not None:
        stash.reservation_released = released
    tags = input_value.get("tpm_limited_tags")
    if tags is not None:
        stash.tpm_limited_tags = frozenset(tags)
