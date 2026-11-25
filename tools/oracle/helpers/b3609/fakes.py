import datetime as dt
from collections.abc import AsyncIterator, Coroutine

from litellm.types.utils import Usage
from pydantic import JsonValue

from .model import (
    FIXED_STAMP,
    SCRIPTED_CLASS,
    FailureRecord,
    ScriptedReadError,
    UsageAndCost,
)
from .recorder import RECORDER


class ScriptedResponse:
    def __init__(self, chunks: list[bytes], failure: ScriptedReadError | None, status: int) -> None:
        self.status_code = status
        self.chunks = chunks
        self.failure = failure

    async def aiter_bytes(self) -> AsyncIterator[bytes]:
        for chunk in self.chunks:
            yield chunk
        if self.failure is not None:
            raise self.failure


class DeferredMarker:
    def __call__(self) -> None: ...


class PartialUsageSink:
    def __call__(self, usage: Usage, response_cost: float) -> None:
        RECORDER.partial = UsageAndCost(usage, response_cost)


class StubLogging:
    def __init__(self, params: dict[str, JsonValue]) -> None:
        self.litellm_params: dict[str, JsonValue] = params
        self.model_call_details: dict[str, JsonValue] = {}
        self.completion_start_time: dt.datetime | None = None
        self._on_deferred_stream_complete: DeferredMarker | None = None
        self._deferred_stream_complete_args: tuple[Coroutine[None, None, None], ...] | None = None
        self.record_partial_usage_for_failure = PartialUsageSink()

    def _update_completion_start_time(self, completion_start_time: dt.datetime) -> None:
        self.completion_start_time = FIXED_STAMP
        self.model_call_details["completion_start_time"] = completion_start_time.isoformat()

    async def dispatch_failure_handlers(
        self,
        exception: Exception,
        traceback_exception: str,
        *,
        prefer_async_handlers: bool = False,
    ) -> None:
        _ = prefer_async_handlers
        lines = traceback_exception.rstrip().splitlines()
        tail = "" if not lines else lines[-1]
        qualified = f"{type(exception).__module__}.{SCRIPTED_CLASS}"
        record = FailureRecord(str(exception), tail.replace(qualified, SCRIPTED_CLASS))
        RECORDER.failures.append(record)
