from collections.abc import Sequence

from litellm.proxy.hooks.batch_enqueued_tokens import BatchEnqueuedTokenReservation
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    ReservationAwareIncrementOperation as PipelineOperation,
)
from pydantic import JsonValue

from .inputs import HiddenParams


def require_no_span(span: JsonValue, subject: str) -> None:
    if span is not None:
        raise RuntimeError(f"{subject} carry no parent span")


class RecordingCache:
    def __init__(self, values: dict[str, JsonValue], gauge_keys: set[str]) -> None:
        self.dual_cache = RecordingDualCache(self)
        self.values = values
        self.gauge_keys = gauge_keys
        self.gauge_reads: list[JsonValue] = []
        self.gauge_writes: list[JsonValue] = []
        self.increments: list[JsonValue] = []
        self.pipelines: list[JsonValue] = []
        self.local_sets: list[JsonValue] = []

    async def async_get_cache(
        self,
        *,
        key: str,
        litellm_parent_otel_span: JsonValue = None,
        local_only: bool = False,
    ) -> JsonValue:
        require_no_span(litellm_parent_otel_span, "the settlement cache reads")
        if not local_only:
            raise RuntimeError("the settlement cache reads local counters only")
        if key in self.gauge_keys:
            self.gauge_reads.append(key)
        return self.values.get(key)

    async def async_set_cache(
        self,
        *,
        key: str,
        value: JsonValue,
        ttl: int,
        litellm_parent_otel_span: JsonValue = None,
        local_only: bool = False,
    ) -> None:
        require_no_span(litellm_parent_otel_span, "the settlement cache writes")
        if not local_only:
            raise RuntimeError("the settlement cache writes local counters only")
        record: JsonValue = {"key": key, "value": value, "ttl": ttl}
        if key in self.gauge_keys:
            self.gauge_writes.append(record)
        else:
            self.local_sets.append(record)
        self.values[key] = value

    async def async_increment_cache(
        self,
        *,
        key: str,
        value: float,
        litellm_parent_otel_span: JsonValue = None,
        ttl: int | None = None,
    ) -> None:
        require_no_span(litellm_parent_otel_span, "the counter increments")
        operation: PipelineOperation = {"key": key, "increment_value": value, "ttl": ttl}
        self.increments.append(operation_json(operation))


class RecordingDualCache:
    redis_cache: JsonValue = None

    def __init__(self, owner: RecordingCache) -> None:
        self.owner = owner

    async def async_increment_cache_pipeline(
        self,
        *,
        increment_list: Sequence[PipelineOperation],
        litellm_parent_otel_span: JsonValue = None,
    ) -> None:
        require_no_span(litellm_parent_otel_span, "the pipeline writes")
        batch: list[JsonValue] = [operation_json(operation) for operation in increment_list]
        self.owner.pipelines.append(batch)

    async def async_increment_cache_pipeline_post_call(  # ast-grep-ignore: no-long-names
        self,
        increment_list: Sequence[PipelineOperation],
        parent_otel_span: JsonValue = None,
    ) -> None:
        require_no_span(parent_otel_span, "the post-call pipeline writes")
        await self.async_increment_cache_pipeline(increment_list=increment_list)


class FakeBatchStore:
    def __init__(self, popped: BatchEnqueuedTokenReservation | None) -> None:
        self.calls: list[JsonValue] = []
        self.popped = popped

    async def save_reservation(
        self, *, batch_id: str, reservation: BatchEnqueuedTokenReservation, **_span: JsonValue
    ) -> None:
        self.calls.append(
            {"call": "save", "batch_id": batch_id, "tokens": reservation.tokens}
        )

    async def pop_reservation(
        self, *, batch_id: str, **_span: JsonValue
    ) -> BatchEnqueuedTokenReservation | None:
        self.calls.append({"call": "pop", "batch_id": batch_id})
        return self.popped

    async def refund(
        self, *, reservation: BatchEnqueuedTokenReservation, **_span: JsonValue
    ) -> None:
        self.calls.append(
            {"call": "refund", "tokens": reservation.tokens}
        )


class HiddenResponse:
    def __init__(self, headers: dict[str, JsonValue]) -> None:
        self._hidden_params: HiddenParams = {"additional_headers": headers}

    def additional_headers(self) -> dict[str, JsonValue]:
        hidden = self._hidden_params
        return hidden["additional_headers"]


class GroupMap(dict[str, str]):
    def __call__(self, model: str) -> str | None:
        return self.get(model)


def operation_json(operation: PipelineOperation) -> dict[str, JsonValue]:
    return {
        "key": operation["key"],
        "increment_value": float(operation["increment_value"]),
        "ttl": operation["ttl"],
    }
