import json
from datetime import UTC, datetime
from typing import Literal, TypeAlias, TypedDict, cast

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy._types import (
    PassThroughEndpointLoggingTypedDict as LoggingOutcome,
)
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

Operation: TypeAlias = Literal["gemini_handler", "gemini_chunks", "vertex_chunks"]


class Input(TypedDict, total=False):
    operation: Operation
    url_route: str
    body: JsonValue
    request_body: dict[str, JsonValue] | None
    lines: list[str]
    model: str | None


class ChoiceMessage(BaseModel):
    content: str | None = None


class ChoiceRow(BaseModel):
    finish_reason: str | None = None
    message: ChoiceMessage | None = None


class UsageCounts(BaseModel):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class ChatDump(BaseModel):
    id: str | None = None
    model: str | None = None
    choices: list[ChoiceRow] = []
    usage: UsageCounts | None = None


class VideoDump(BaseModel):
    id: str | None = None
    model: str | None = None
    status: str | None = None


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


INPUT: TypeAdapter[Input] = TypeAdapter(Input)
BODY: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
REQUEST_BODY: TypeAdapter[dict[str, JsonValue] | None] = TypeAdapter(dict[str, JsonValue] | None)

START: datetime = datetime(2024, 1, 1, tzinfo=UTC)


class Recorder:
    def __init__(self) -> None:
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
        match completion_response:
            case VideoObject():
                self.calls.append(
                    RecordedCall(
                        model=model,
                        custom_llm_provider=custom_llm_provider,
                        response_model=completion_response.model,
                        usage=None,
                        kind="video",
                        call_type=call_type,
                        vertex_location=vertex_location,
                    )
                )
            case ModelResponse():
                chat = ChatDump.model_validate_json(completion_response.model_dump_json())
                usage = chat.usage
                counts = (
                    (usage.prompt_tokens, usage.completion_tokens, usage.total_tokens)
                    if usage is not None
                    else None
                )
                self.calls.append(
                    RecordedCall(
                        model=model,
                        custom_llm_provider=custom_llm_provider,
                        response_model=completion_response.model,
                        usage=counts,
                        kind="completion",
                        call_type=call_type,
                        vertex_location=vertex_location,
                    )
                )
        return self.price


def make_logging() -> Logging:
    logging_obj = Logging(
        model="fixture-model",
        messages=None,
        stream=False,
        call_type="pass_through_endpoint",
        start_time=START,
        litellm_call_id="call-1",
        function_id="function-1",
    )
    logging_obj.update_environment_variables(litellm_params={}, optional_params={})
    return logging_obj


def result_value(response: ModelResponse | VideoObject | None) -> JsonValue:
    if response is None:
        return None
    match response:
        case VideoObject():
            video = VideoDump.model_validate_json(response.model_dump_json())
            return {
                "kind": "video",
                "id": video.id,
                "model": video.model,
                "status": video.status,
                "choices": [],
                "usage": None,
            }
        case ModelResponse():
            chat = ChatDump.model_validate_json(response.model_dump_json())
            usage: JsonValue = None
            if chat.usage is not None:
                usage = {
                    "prompt_tokens": chat.usage.prompt_tokens,
                    "completion_tokens": chat.usage.completion_tokens,
                    "total_tokens": chat.usage.total_tokens,
                }
            choices: list[JsonValue] = [
                {
                    "finish_reason": row.finish_reason,
                    "content": row.message.content if row.message is not None else None,
                }
                for row in chat.choices
            ]
            return {
                "kind": "chat",
                "id": chat.id,
                "model": chat.model,
                "status": None,
                "choices": choices,
                "usage": usage,
            }


def kwargs_value(outcome: LoggingOutcome) -> dict[str, JsonValue]:
    kwargs = cast("dict[str, JsonValue]", outcome["kwargs"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L5066
    return {
        "model": kwargs.get("model"),
        "custom_llm_provider": kwargs.get("custom_llm_provider"),
        "response_cost": kwargs.get("response_cost"),
    }


def logging_value(logging_obj: Logging) -> dict[str, JsonValue]:
    logged = RecordedLogging.model_validate(logging_obj)
    details = logged.model_call_details
    optional = logged.optional_params
    return {
        "model": logged.model,
        "custom_llm_provider": logged.custom_llm_provider,
        "call_type": logged.call_type,
        "details": {
            "model": details.model,
            "custom_llm_provider": details.custom_llm_provider,
            "response_cost": details.response_cost,
        },
        "optional": {"vertex_location": optional.get("vertex_location")},
    }


def cost_value(recorder: Recorder) -> list[JsonValue]:
    return [
        {
            "model": call.model,
            "custom_llm_provider": call.custom_llm_provider,
            "response_model": call.response_model,
            "usage": list(call.usage) if call.usage is not None else None,
            "kind": call.kind,
            "call_type": call.call_type,
            "vertex_location": call.vertex_location,
        }
        for call in recorder.calls
    ]


def answer_value(call: Input, recorder: Recorder) -> JsonValue:
    logging_obj = make_logging()
    empty_body: dict[str, JsonValue] = {}
    if call["operation"] == "gemini_handler":
        body = BODY.validate_python(call.get("body"))
        request_body = REQUEST_BODY.validate_python(call.get("request_body"))
        response = httpx.Response(200, content=json.dumps(body).encode())
        outcome = GeminiHandler.gemini_passthrough_handler(
            httpx_response=response,
            response_body=body,
            logging_obj=logging_obj,
            url_route=call["url_route"],
            result="",
            start_time=START,
            end_time=START,
            cache_hit=False,
            request_body=request_body if request_body is not None else {},
        )
    elif call["operation"] == "gemini_chunks":
        outcome = GeminiHandler._handle_logging_gemini_collected_chunks(  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/gemini_passthrough_logging_handler.py#L132
            litellm_logging_obj=logging_obj,
            passthrough_success_handler_obj=PassThroughEndpointLogging(),
            url_route=call["url_route"],
            request_body=empty_body,
            endpoint_type=EndpointType.GEMINI,
            start_time=START,
            all_chunks=call["lines"],
            model=call.get("model"),
            end_time=START,
        )
    else:
        outcome = VertexHandler._handle_logging_vertex_collected_chunks(  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/vertex_passthrough_logging_handler.py#L562
            litellm_logging_obj=logging_obj,
            passthrough_success_handler_obj=PassThroughEndpointLogging(),
            url_route=call["url_route"],
            request_body=empty_body,
            endpoint_type=EndpointType.VERTEX_AI,
            start_time=START,
            all_chunks=call["lines"],
            model=call.get("model"),
            end_time=START,
        )
    return {
        "result": result_value(
            cast(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L5066
                "ModelResponse | VideoObject | None", outcome["result"]
            )
        ),
        "kwargs": kwargs_value(outcome),
        "logging": logging_value(logging_obj),
        "cost": cost_value(recorder),
    }


def run(input_value: JsonValue) -> JsonValue:
    call = INPUT.validate_python(input_value)
    recorder = Recorder()
    litellm.completion_cost = recorder  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/cost_calculator.py#L1332
    try:
        return answer_value(call, recorder)
    except (AttributeError, KeyError, TypeError, ValueError):
        return {"failed": True}
