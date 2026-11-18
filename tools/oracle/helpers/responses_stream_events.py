from collections.abc import Callable
from datetime import datetime
from typing import Final, Literal, NotRequired, TypedDict, cast

import httpx
import litellm
from litellm.caching.caching import Cache
from litellm.llms.base_llm.responses.transformation import BaseResponsesAPIConfig
from litellm.llms.openai.responses.transformation import OpenAIResponsesAPIConfig
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from litellm.responses import streaming_iterator
from litellm.responses.streaming_iterator import (
    SyncResponsesAPIStreamingIterator as SyncIterator,
)
from litellm.types.llms.openai import (
    ResponseAPIUsage,
    ResponsesAPIResponse,
    ResponsesAPIStreamingResponse,
)
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

class ModelInfo(TypedDict):
    id: str


class Metadata(TypedDict):
    model_info: NotRequired[ModelInfo]
    encrypted_content_affinity_enabled: NotRequired[bool]  # ast-grep-ignore: no-long-names


class RequestData(TypedDict):
    input: NotRequired[str]


class Input(TypedDict):
    payloads: list[str]
    model: str
    provider: str | None
    request_data: RequestData
    metadata: Metadata
    transform: Literal["configured", "absent"]
    estimate: Literal["fixed", "absent"]
    cost: Literal["positive", "zero", "missing", "error"]
    cache: Literal["off", "failing"]


class Estimate:
    def __init__(self, records: list[JsonValue], mode: Literal["fixed", "absent"]) -> None:
        self.records = records
        self.mode = mode

    def __call__(
        self,
        model: str,
        request_input: JsonValue,
        responses_api_request: JsonValue,
        generated_text: str,
    ) -> ResponseAPIUsage | None:
        if self.mode == "absent":
            return None
        self.records.append(
            ["estimate", model, request_input, responses_api_request, generated_text]
        )
        input_tokens = 1000 + len(model)
        output_tokens = len(generated_text)
        return ResponseAPIUsage(
            input_tokens=input_tokens,
            output_tokens=output_tokens,
            total_tokens=input_tokens + output_tokens,
        )


class InlineExecutor:
    def submit(self, function: Callable[..., None], *args: JsonValue, **kwargs: JsonValue) -> None:
        function(*args, **kwargs)


class CacheHandler:
    def __init__(self) -> None:
        self.request_kwargs: dict[str, JsonValue] = {"stream": True}
        self.preset_cache_key: str | None = None
        self.original_function = "aresponses"
        self.dual_cache: JsonValue = None

    def _should_store_result_in_cache(
        self, original_function: str | None, kwargs: dict[str, JsonValue]
    ) -> bool:
        return True


class FailingCache(Cache):
    def add_cache(self, result: str, **kwargs: JsonValue) -> None:
        raise RuntimeError("cache write failed")


class StubLogging:
    def __init__(
        self,
        records: list[JsonValue],
        cost: Literal["positive", "zero", "missing", "error"],
        cache_handler: CacheHandler | None,
    ) -> None:
        self.records = records
        self.cost = cost
        self.model_call_details: dict[str, JsonValue] = {}
        self.start_time = datetime(2026, 1, 1)
        self.completion_start_time: datetime | None = None
        self._llm_caching_handler = cache_handler

    def _update_completion_start_time(self, completion_start_time: datetime) -> None:
        self.completion_start_time = completion_start_time

    async def async_success_handler(
        self,
        *,
        result: JsonValue,
        start_time: datetime,
        end_time: datetime,
        cache_hit: bool | None,
    ) -> None:
        self.records.append("success_async")

    def success_handler(
        self,
        *,
        result: JsonValue,
        cache_hit: bool | None,
        start_time: datetime,
        end_time: datetime,
    ) -> None:
        self.records.append("success_sync")

    async def async_failure_handler(
        self,
        exception: BaseException,
        traceback_exception: str,
        start_time: datetime,
        end_time: datetime,
    ) -> None:
        self.records.append("failure_async")

    def failure_handler(
        self,
        exception: BaseException,
        traceback_exception: str,
        start_time: datetime,
        end_time: datetime,
    ) -> None:
        self.records.append("failure_sync")

    def _response_cost_calculator(self, *, result: ResponsesAPIResponse) -> float | None:
        self.records.append(["cost", result.id])
        if self.cost == "positive":
            return 0.5
        if self.cost == "zero":
            return 0.0
        if self.cost == "missing":
            return None
        raise RuntimeError("cost calculation failed")


def event_json(event: ResponsesAPIStreamingResponse) -> JsonValue:
    return TypeAdapter(JsonValue).validate_json(event.model_dump_json())


def normalize(value: JsonValue) -> JsonValue:
    match value:
        case dict():
            return {key: normalize(child) for key, child in value.items() if child is not None}
        case list():
            return [normalize(child) for child in value]
        case None | bool() | int() | float() | str():
            return value


def completed_json(iterator: SyncIterator) -> JsonValue:
    completed = iterator.completed_response
    return None if completed is None else event_json(completed)


def projection(
    iterator: SyncIterator,
    events: list[JsonValue],
    error: str | None,
    records: list[JsonValue],
) -> JsonValue:
    state: dict[str, JsonValue] = {
        "finished": iterator.finished,
        "failure_handled": iterator._failure_handled,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L316-L324
        "logged": iterator._completed_response_logged,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L316-L324
        "cached": iterator._completed_response_cached,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L316-L324
        "completion_start_set": iterator.logging_obj.completion_start_time is not None,
        "completed": completed_json(iterator),
        "generated_content": iterator._generated_content,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L316-L324
        "generated_tool_arguments": iterator._generated_tool_arguments,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L316-L324
    }
    return {
        "events": events,
        "error": error,
        "state": state,
        "records": records,
    }


def run(input_value: Input) -> JsonValue:
    records: list[JsonValue] = []
    cache_handler = CacheHandler() if input_value["cache"] == "failing" else None
    logging_obj = StubLogging(records, input_value["cost"], cache_handler)
    estimate = Estimate(records, input_value["estimate"])
    provider: OpenAIResponsesAPIConfig | None = (
        OpenAIResponsesAPIConfig() if input_value["transform"] == "configured" else None
    )
    iterator = SyncIterator(
        response=httpx.Response(200, headers={}),
        model=input_value["model"],
        responses_api_provider_config=cast("BaseResponsesAPIConfig", provider),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L1113-L1128
        logging_obj=cast("LiteLLMLoggingObj", logging_obj),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L556
        litellm_metadata=dict(input_value["metadata"]),
        custom_llm_provider=input_value["provider"],
        request_data=dict(input_value["request_data"]),
        call_type="responses",
    )
    original_estimate = streaming_iterator._estimate_usage_safely  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L1564-L1580
    original_executor = streaming_iterator.executor
    original_cache = litellm.cache
    streaming_iterator._estimate_usage_safely = estimate  # type: ignore[assignment]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L1564-L1580
    streaming_iterator.executor = InlineExecutor()  # type: ignore[assignment, misc]  # pyright: ignore[reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L551-L560
    if input_value["cache"] == "failing":
        litellm.cache = FailingCache()
    try:
        events: list[JsonValue] = []
        error: str | None = None
        for payload in input_value["payloads"]:
            try:
                event = iterator._process_chunk(payload)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L363-L529
            except Exception as failure:  # noqa: BLE001 - the raised message is the compared outcome
                error = str(failure)
                break
            events.append(None if event is None else event_json(event))
        return normalize(projection(iterator, events, error, records))
    finally:
        streaming_iterator._estimate_usage_safely = original_estimate  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L1564-L1580
        streaming_iterator.executor = original_executor
        litellm.cache = original_cache
