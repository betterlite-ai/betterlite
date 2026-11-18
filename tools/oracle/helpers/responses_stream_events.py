from datetime import UTC, datetime
from typing import Literal, NotRequired, Protocol, TypedDict, cast

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from litellm.llms.base_llm.responses.transformation import BaseResponsesAPIConfig
from litellm.llms.openai.responses.transformation import OpenAIResponsesAPIConfig
from litellm.responses import streaming_iterator
from litellm.responses.streaming_iterator import (
    SyncResponsesAPIStreamingIterator as SyncIterator,
)
from litellm.types.llms.openai import ResponseAPIUsage, ResponsesAPIResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

litellm.suppress_debug_info = True


class ModelInfo(TypedDict):
    id: str


class Metadata(TypedDict):
    model_info: NotRequired[ModelInfo]
    # ast-grep-ignore: no-long-names
    encrypted_content_affinity_enabled: NotRequired[bool]


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
    cost: Literal["positive", "missing"]


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


class Handler(Protocol):
    def __call__(self, *args: JsonValue, **kwargs: JsonValue) -> None: ...


class ExecutorPort(Protocol):
    def submit(self, function: Handler, *args: JsonValue, **kwargs: JsonValue) -> None: ...


class InlineExecutor(ExecutorPort):
    @override
    def submit(self, function: Handler, *args: JsonValue, **kwargs: JsonValue) -> None:
        function(*args, **kwargs)


class StubLogging:
    def __init__(
        self,
        records: list[JsonValue],
        cost: Literal["positive", "missing"],
    ) -> None:
        self.records = records
        self.cost = cost
        self.model_call_details: dict[str, JsonValue] = {}
        self.start_time = datetime(2026, 1, 1, tzinfo=UTC)
        self.completion_start_time: datetime | None = None

    def _update_completion_start_time(self, completion_start_time: datetime) -> None:
        self.completion_start_time = completion_start_time
        self.model_call_details["completion_start_time"] = completion_start_time.isoformat()

    async def async_success_handler(self, *args: JsonValue, **kwargs: JsonValue) -> None:
        del args, kwargs
        self.records.append("success_async")

    def success_handler(self, *args: JsonValue, **kwargs: JsonValue) -> None:
        del args, kwargs
        self.records.append("success_sync")

    async def async_failure_handler(self, *args: JsonValue, **kwargs: JsonValue) -> None:
        del args, kwargs
        self.records.append("failure_async")

    def failure_handler(self, *args: JsonValue, **kwargs: JsonValue) -> None:
        del args, kwargs
        self.records.append("failure_sync")

    def _response_cost_calculator(self, *, result: ResponsesAPIResponse) -> float | None:
        self.records.append(["cost", result.id])
        if self.cost == "positive":
            return 0.5
        return None


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
    if completed is None:
        return None
    return TypeAdapter(JsonValue).validate_json(completed.model_dump_json())


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
    logging_obj = StubLogging(records, input_value["cost"])
    estimate = Estimate(records, input_value["estimate"])
    provider: OpenAIResponsesAPIConfig | None = (
        OpenAIResponsesAPIConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/responses/transformation.py#L105
        if input_value["transform"] == "configured"
        else None
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
    original_executor = streaming_iterator.executor  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/thread_pool_executor.py#L79
    streaming_iterator._estimate_usage_safely = estimate  # type: ignore[assignment]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L1564-L1580
    streaming_iterator.executor = InlineExecutor()  # type: ignore[attr-defined]  # pyright: ignore[reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/thread_pool_executor.py#L79
    try:
        events: list[JsonValue] = []
        error: str | None = None
        for payload in input_value["payloads"]:
            try:
                event = iterator._process_chunk(payload)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L363-L532
            except Exception as failure:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L363-L532
                error = str(failure)
                break
            events.append(
                None
                if event is None
                else TypeAdapter(JsonValue).validate_json(event.model_dump_json())
            )
        return normalize(projection(iterator, events, error, records))
    finally:
        streaming_iterator._estimate_usage_safely = original_estimate  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/streaming_iterator.py#L1564-L1580
        streaming_iterator.executor = original_executor  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/thread_pool_executor.py#L79
