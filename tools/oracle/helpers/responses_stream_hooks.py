import asyncio
from collections.abc import Callable, Coroutine
from concurrent.futures import Future, ThreadPoolExecutor
from datetime import datetime
from typing import Literal, ParamSpec, TypedDict, TypeVar, cast

import httpx
import litellm
from litellm.caching.caching import Cache
from litellm.exceptions import (
    AuthenticationError,
    BadRequestError,
    MidStreamFallbackError,
    NotFoundError,
)
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.responses import streaming_iterator
from litellm.responses.streaming_iterator import BaseResponsesAPIStreamingIterator as StreamBase
from litellm.types.llms.openai import ResponseCompletedEvent, ResponseFailedEvent
from pydantic import BaseModel, Field, JsonValue, TypeAdapter
from typing_extensions import override

Operation = Literal["cache", "logging", "headers", "failed", "chunk", "metadata", "failure"]
Params = ParamSpec("Params")
Result = TypeVar("Result")
JSON = TypeAdapter(JsonValue)


class Input(TypedDict):
    operation: Operation
    rows: list[dict[str, JsonValue]]


class Row(BaseModel):
    event: ResponseCompletedEvent | ResponseFailedEvent
    kwargs: dict[str, JsonValue] | None
    preset: str | None
    request: dict[str, JsonValue]
    details: dict[str, JsonValue]
    processed: dict[str, JsonValue]
    raw: dict[str, str]
    call_type: str | None
    logging_type: str | None
    actions: list[str]
    eligible: bool
    broken: bool
    deferred: bool
    cached: bool
    yielded: bool


class InlineExecutor(ThreadPoolExecutor):
    @override
    def submit(
        self, fn: Callable[Params, Result], /, *args: Params.args, **kwargs: Params.kwargs,
    ) -> Future[Result]:
        future: Future[Result] = Future()
        future.set_result(fn(*args, **kwargs))
        return future


class Recorder:
    def __init__(self, row: Row) -> None:
        self.row = row
        self.records: list[JsonValue] = []
        self.start_time = datetime.fromisoformat("2026-01-01T00:00:00+00:00")
        self.model_call_details = dict(row.details)
        self.call_type = row.logging_type
        self.request_kwargs = row.kwargs
        self.preset_cache_key = row.preset
        self.original_function = "aresponses"
        self.dual_cache = None
        self._llm_caching_handler = self
        self._on_deferred_stream_complete = True if row.deferred else None
        self._deferred_stream_complete_args: tuple[Coroutine[None, None, None], ...] | None = None
        self.cache_done = asyncio.Event()
        self.success_done = asyncio.Event()

    def close_deferred(self) -> None:
        if self._deferred_stream_complete_args is not None:
            for pending in self._deferred_stream_complete_args:
                pending.close()

    def _should_store_result_in_cache(
        self, original_function: str | None, kwargs: dict[str, JsonValue],
    ) -> bool:
        self.records.append(["eligible", original_function, kwargs])
        return self.row.eligible

    async def async_add_cache(
        self, payload: str, *, dynamic_cache_object: None, **kwargs: JsonValue,
    ) -> None:
        del dynamic_cache_object
        parsed = TypeAdapter(dict[str, JsonValue]).validate_json(payload)
        self.records.append(["cache", kwargs, parsed["id"]])
        self.cache_done.set()
        if self.row.broken:
            raise ValueError("broken")

    async def dispatch_success_handlers(
        self, result: BaseModel, *, start_time: datetime, end_time: datetime,
        cache_hit: bool | None, prefer_async_handlers: bool,
    ) -> None:
        del result, start_time, end_time
        self.records.append(["success", cache_hit, prefer_async_handlers])
        self.success_done.set()

    async def async_failure_handler(
        self, exception: Exception, traceback_exception: str,
        start_time: datetime, end_time: datetime,
    ) -> None:
        del traceback_exception, start_time, end_time
        status = FailureView.model_validate(exception, from_attributes=True).status_code
        self.records.append(["async_failure", status])
        if self.row.broken:
            raise ValueError("broken")

    def failure_handler(
        self, exception: Exception, traceback_exception: str,
        start_time: datetime, end_time: datetime,
    ) -> None:
        del traceback_exception, start_time, end_time
        status = FailureView.model_validate(exception, from_attributes=True).status_code
        self.records.append(["sync_failure", status])

    def _response_cost_calculator(self, result: BaseModel) -> int:
        del result
        self.records.append(["cost"])
        return 1

    def metadata(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L833
        self, *, result: BaseModel, logging_obj: Logging, model: str, kwargs: dict[str, JsonValue],
        start_time: datetime, end_time: datetime, include_overhead: bool,
    ) -> None:
        del result, logging_obj, model, start_time, end_time
        self.records.append(["metadata", kwargs, include_overhead])

    async def post(
        self, *, request_data: dict[str, JsonValue], response: BaseModel, call_type: str,
    ) -> None:
        del response
        self.records.append(["post", request_data, call_type])


class FailureView(BaseModel):
    status_code: int


class ChunkProbe:
    def __init__(self, action: str, recorder: Recorder) -> None:
        self.action = action
        self.recorder = recorder
        self.async_post_call_streaming_deployment_hook = self.chunk

    async def chunk(
        self, request_data: dict[str, JsonValue],
        response_chunk: ResponseCompletedEvent | ResponseFailedEvent, call_type: str | None,
    ) -> ResponseCompletedEvent | ResponseFailedEvent | None:
        self.recorder.records.append(["chunk", self.action, call_type, request_data])
        if self.action == "fail":
            raise ValueError("broken")
        if self.action == "keep":
            return None
        response_chunk.response.id = self.action
        return response_chunk


class Probe(StreamBase):
    def configure(self, row: Row) -> None:
        self.completed_response = row.event
        self._completed_response_cache_hit = True if row.cached else None
        self._persist_completed_response_before_logging = not row.cached
        self._yielded_first_chunk = row.yielded
        self._generated_content = "text"
        self._generated_tool_arguments = "tool"
        self._hidden_params["additional_headers"] = row.processed
        self._raw_response_headers = row.raw

    def failure_result(self, recorder: Recorder) -> JsonValue:
        row = recorder.row
        try:
            self._maybe_raise_for_error_event(row.event)
        except MidStreamFallbackError as error:
            return {
                "status": error.status_code, "pre": error.is_pre_first_chunk,
                "original": error.original_exception is not None,
                "billed": "combined_usage_object" in recorder.model_call_details,
            }
        except (AuthenticationError, BadRequestError, NotFoundError) as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L664
            return {
                "status": FailureView.model_validate(error, from_attributes=True).status_code,
                "pre": None, "original": False,
                "billed": "combined_usage_object" in recorder.model_call_details,
            }
        return None

    def report_failure(self, row: Row) -> None:
        try:
            self._maybe_raise_for_error_event(row.event)
        except (AuthenticationError, BadRequestError, MidStreamFallbackError, NotFoundError) as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L664
            self._handle_failure(error)
            self._handle_failure(error)

    async def invoke(self, operation: Operation, recorder: Recorder) -> JsonValue:
        row = recorder.row
        extra: JsonValue = None
        if operation == "cache":
            self._persist_completed_response_to_cache(is_async=True)
            self._persist_completed_response_to_cache(is_async=True)
        elif operation == "logging":
            self._log_completed_response(is_async=True)
            self._log_completed_response(is_async=True)
            extra = {"deferred": recorder._deferred_stream_complete_args is not None}  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L560
        elif operation == "headers":
            copy = row.event.model_copy(deep=True)
            self._restore_provider_response_headers(copy)
            extra = JSON.validate_python(copy.response._hidden_params)  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportUnknownMemberType, reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L607
        elif operation == "failed":
            self._handle_logging_failed_response()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L616
            extra = self.failure_result(recorder)
        elif operation == "chunk":
            result = await self._call_post_streaming_deployment_hook(row.event)
            view = ChunkView.model_validate(result, from_attributes=True)
            extra = {"id": view.response.id, "ran": view.ran}
        elif operation == "metadata":
            self._run_post_success_hooks(recorder.start_time)
        elif operation == "failure":
            self.report_failure(row)
        await self._await_pending_logging()
        if self._completed_response_cached:
            await recorder.cache_done.wait()
        if self._completed_response_logged and not row.deferred:
            await recorder.success_done.wait()
        return {
            "records": recorder.records, "cached": self._completed_response_cached,
            "logged": self._completed_response_logged, "failed": self._failure_handled,
            "extra": extra,
        }


class ResponseView(BaseModel):
    id: str


class ChunkView(BaseModel):
    response: ResponseView
    ran: bool = Field(default=False, alias="_post_streaming_hooks_ran")


async def answer(operation: Operation, values: dict[str, JsonValue]) -> JsonValue:
    row = Row.model_validate(values)
    recorder = Recorder(row)
    probe = Probe(
        response=httpx.Response(200), model="gpt-4o", responses_api_provider_config=None,
        logging_obj=cast(Logging, recorder),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L305
        custom_llm_provider="openai", call_type=row.call_type,
    )
    probe.request_data.update(row.request)
    probe.configure(row)
    saved = (  # type: ignore[misc]  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L49
        litellm.cache,
        litellm.callbacks,  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L777
        streaming_iterator.executor,  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L34
        streaming_iterator.update_response_metadata,  # type: ignore[attr-defined, misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L32
        streaming_iterator.async_post_call_success_deployment_hook,  # type: ignore[attr-defined, misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L49
    )
    try:
        litellm.cache = cast(Cache, recorder)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L728
        litellm.callbacks = [
            cast(CustomLogger, ChunkProbe(action, recorder))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L780
            for action in row.actions
        ]
        streaming_iterator.executor = InlineExecutor()  # type: ignore[attr-defined, assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L34
        streaming_iterator.update_response_metadata = recorder.metadata  # type: ignore[attr-defined, assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L832
        streaming_iterator.async_post_call_success_deployment_hook = recorder.post  # type: ignore[attr-defined, assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L868
        return await probe.invoke(operation, recorder)
    finally:
        recorder.close_deferred()
        (
            litellm.cache,
            litellm.callbacks,
            streaming_iterator.executor,  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L34
            streaming_iterator.update_response_metadata,  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L32
            streaming_iterator.async_post_call_success_deployment_hook,  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L49
        ) = saved  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L49


def run(input_data: Input) -> JsonValue:
    answers = [asyncio.run(answer(input_data["operation"], row)) for row in input_data["rows"]]
    return {"rows": answers}
