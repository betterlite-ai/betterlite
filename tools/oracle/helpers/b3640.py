import json
from datetime import UTC, datetime
from typing import Literal, TypedDict, cast

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy.pass_through_endpoints.llm_provider_handlers.gemini_passthrough_logging_handler import (  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/gemini_passthrough_logging_handler.py#L34-L35
    GeminiPassthroughLoggingHandler as GeminiHandler,
)
from litellm.proxy.pass_through_endpoints.llm_provider_handlers.vertex_passthrough_logging_handler import (  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/vertex_passthrough_logging_handler.py#L333-L338
    VertexPassthroughLoggingHandler as VertexHandler,
)
from litellm.proxy.pass_through_endpoints.success_handler import PassThroughEndpointLogging
from litellm.types.passthrough_endpoints.pass_through_endpoints import EndpointType
from litellm.types.utils import ModelResponse
from litellm.types.videos.main import VideoObject
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

litellm.suppress_debug_info = True


class Input(TypedDict):
    operation: Literal["gemini_handler", "gemini_chunks", "vertex_chunks"]
    url_route: str
    body: JsonValue
    request_body: dict[str, JsonValue] | None
    lines: list[str]
    model: str | None


class ChoiceMessage(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    content: str | None = None


class ChoiceRow(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    finish_reason: str | None = None
    message: ChoiceMessage | None = None


class UsageCounts(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class ChatDump(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    id: str | None = None
    model: str | None = None
    choices: list[ChoiceRow] = []
    usage: UsageCounts | None = None


class VideoDump(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    id: str | None = None
    model: str | None = None
    status: str | None = None


class ResponseModel(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    model: str | None = None


class LoggingDetails(BaseModel):
    model: str | None = None
    custom_llm_provider: str | None = None
    response_cost: float | None = None


class RecordedLogging(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    model: str | None = None
    call_type: str
    custom_llm_provider: str | None = None
    model_call_details: LoggingDetails
    optional_params: dict[str, JsonValue]


class RecordedCall(BaseModel):
    model: str
    custom_llm_provider: str
    response_model: str | None
    usage: tuple[int, int, int] | None
    kind: str
    call_type: str | None
    vertex_location: str | None


class ChoiceView(BaseModel):
    finish_reason: str | None = None
    content: str | None = None


class ResultView(BaseModel):
    kind: str
    id: str | None = None
    model: str | None = None
    status: str | None = None
    choices: list[ChoiceView] = []
    usage: UsageCounts | None = None


class LoggingView(BaseModel):
    model: str | None = None
    custom_llm_provider: str | None = None
    call_type: str
    details: LoggingDetails
    optional: dict[str, JsonValue]


INPUT: TypeAdapter[Input] = TypeAdapter(Input)
BODY: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
REQUEST_BODY: TypeAdapter[dict[str, JsonValue] | None] = TypeAdapter(dict[str, JsonValue] | None)
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

START: datetime = datetime(2024, 1, 1, tzinfo=UTC)


class Recorder:
    def __init__(self, *, video: bool) -> None:
        self.video = video
        self.price = 1.25
        self.calls: list[RecordedCall] = []

    def __call__(
        self,
        completion_response: ModelResponse | VideoObject,
        model: str,
        custom_llm_provider: str,
        call_type: str | None = None,
        vertex_location: str | None = None,
    ) -> float:
        answer = ResponseModel.model_validate(completion_response)
        counts = None
        if not self.video:
            chat = ChatDump.model_validate(completion_response)
            counts = chat.usage
        usage = (
            (counts.prompt_tokens, counts.completion_tokens, counts.total_tokens)
            if counts is not None
            else None
        )
        self.calls.append(
            RecordedCall(
                model=model,
                custom_llm_provider=custom_llm_provider,
                response_model=answer.model,
                usage=usage,
                kind="video" if self.video else "completion",
                call_type=call_type,
                vertex_location=vertex_location,
            )
        )
        return self.price


def result_value(response: ModelResponse | VideoObject | None, *, video: bool) -> JsonValue:
    if response is None:
        return None
    if video:
        recorded = VideoDump.model_validate(response)
        view = ResultView(
            kind="video", id=recorded.id, model=recorded.model, status=recorded.status
        )
        return JSON.validate_json(view.model_dump_json())
    chat = ChatDump.model_validate(response)
    rows = [
        ChoiceView(
            finish_reason=row.finish_reason,
            content=row.message.content if row.message is not None else None,
        )
        for row in chat.choices
    ]
    view = ResultView(
        kind="chat", id=chat.id, model=chat.model, choices=rows, usage=chat.usage
    )
    return JSON.validate_json(view.model_dump_json())


def kwargs_value(kwargs: dict[str, JsonValue]) -> dict[str, JsonValue]:
    return {
        "model": kwargs.get("model"),
        "custom_llm_provider": kwargs.get("custom_llm_provider"),
        "response_cost": kwargs.get("response_cost"),
    }


def logging_value(logging_obj: Logging) -> JsonValue:
    logged = RecordedLogging.model_validate(logging_obj)
    optional: dict[str, JsonValue] = {
        "vertex_location": logged.optional_params.get("vertex_location")
    }
    view = LoggingView(
        model=logged.model,
        custom_llm_provider=logged.custom_llm_provider,
        call_type=logged.call_type,
        details=logged.model_call_details,
        optional=optional,
    )
    return JSON.validate_json(view.model_dump_json())


def dispatched(call: Input, recorder: Recorder) -> dict[str, JsonValue]:
    logging_obj = Logging(
        model="fixture-model",
        messages=None,
        stream=False,
        call_type="pass_through_endpoint",
        start_time=START,
        litellm_call_id="call-1",
        function_id="function-1",
    )
    logging_obj.update_environment_variables(litellm_params={}, optional_params={})  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L910
    empty: dict[str, JsonValue] = {}
    if call["operation"] == "gemini_handler":
        body = BODY.validate_python(call["body"])
        request_body = REQUEST_BODY.validate_python(call["request_body"])
        response = httpx.Response(200, content=json.dumps(body).encode())
        outcome = GeminiHandler.gemini_passthrough_handler(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/gemini_passthrough_logging_handler.py#L40
            httpx_response=response,
            response_body=body,
            logging_obj=logging_obj,
            url_route=call["url_route"],
            result="",
            start_time=START,
            end_time=START,
            cache_hit=False,
            request_body=request_body if request_body is not None else empty,
        )
    elif call["operation"] == "gemini_chunks":
        outcome = GeminiHandler._handle_logging_gemini_collected_chunks(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/gemini_passthrough_logging_handler.py#L132
            litellm_logging_obj=logging_obj,
            passthrough_success_handler_obj=PassThroughEndpointLogging(),
            url_route=call["url_route"],
            request_body=empty,
            endpoint_type=EndpointType.GEMINI,
            start_time=START,
            all_chunks=call["lines"],
            model=call["model"],
            end_time=START,
        )
    else:
        outcome = VertexHandler._handle_logging_vertex_collected_chunks(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/vertex_passthrough_logging_handler.py#L562
            litellm_logging_obj=logging_obj,
            passthrough_success_handler_obj=PassThroughEndpointLogging(),
            url_route=call["url_route"],
            request_body=empty,
            endpoint_type=EndpointType.VERTEX_AI,
            start_time=START,
            all_chunks=call["lines"],
            model=call["model"],
            end_time=START,
        )
    result = cast("ModelResponse | VideoObject | None", outcome["result"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L5066
    kwargs = cast("dict[str, JsonValue]", outcome["kwargs"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L5066
    return {
        "result": result_value(result, video=recorder.video),
        "kwargs": kwargs_value(kwargs),
        "logging": logging_value(logging_obj),
        "cost": [JSON.validate_json(record.model_dump_json()) for record in recorder.calls],
    }


def run(input_value: JsonValue) -> JsonValue:
    call = INPUT.validate_python(input_value)
    recorder = Recorder(video="predictLongRunning" in call["url_route"])
    litellm.completion_cost = recorder  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/cost_calculator.py#L1332
    try:
        return dispatched(call, recorder)
    except (AttributeError, KeyError, TypeError, ValueError):
        return {"failed": True}
