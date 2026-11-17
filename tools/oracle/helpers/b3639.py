import json
from datetime import UTC, datetime
from typing import Literal, TypeAlias, TypedDict, cast

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy._types import (
    PassThroughEndpointLoggingTypedDict as LoggingOutcome,
)
from litellm.proxy.pass_through_endpoints.llm_provider_handlers.vertex_passthrough_logging_handler import (  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/vertex_passthrough_logging_handler.py#L333-L338
    VertexPassthroughLoggingHandler as VertexHandler,
)
from litellm.types.utils import EmbeddingResponse, ImageResponse, Usage
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

Operation: TypeAlias = Literal["predict", "predict_image", "predict_audio", "embed_content"]


class Input(TypedDict):
    operation: Operation
    url_route: str
    status: int
    body: JsonValue
    request_body: JsonValue | None


class DataView(TypedDict):
    data: list[JsonValue]


class StandardView(TypedDict):
    response: JsonValue


class ResponseRef(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    model: str | None = None


class RecordedCall(BaseModel):
    model: str
    custom_llm_provider: str
    vertex_location: str | None
    response_model: str | None


class LoggingDetails(BaseModel):
    model: str | None = None
    custom_llm_provider: str | None = None
    response_cost: float | None = None


class RecordedLogging(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    model: str
    call_type: str
    custom_llm_provider: str | None = None
    model_call_details: LoggingDetails


INPUT: TypeAdapter[Input] = TypeAdapter(Input)
DATA: TypeAdapter[DataView] = TypeAdapter(DataView)
STANDARD: TypeAdapter[StandardView] = TypeAdapter(StandardView)
REQUEST_BODY: TypeAdapter[dict[str, JsonValue] | None] = TypeAdapter(
    dict[str, JsonValue] | None
)


class Recorder:
    def __init__(self) -> None:
        self.price = 0.0007
        self.calls: list[RecordedCall] = []

    def __call__(
        self,
        completion_response: EmbeddingResponse | ImageResponse,
        model: str,
        custom_llm_provider: str,
        vertex_location: str | None = None,
    ) -> float:
        answer = ResponseRef.model_validate(completion_response)
        recorded = RecordedCall(
            model=model,
            custom_llm_provider=custom_llm_provider,
            vertex_location=vertex_location,
            response_model=answer.model,
        )
        self.calls.append(recorded)
        return self.price


def make_logging() -> Logging:
    return Logging(
        model="fixture-model",
        messages=None,
        stream=False,
        call_type="pass_through_endpoint",
        start_time=datetime(2024, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="function-1",
    )


def usage_value(usage: Usage) -> dict[str, JsonValue]:
    details = usage.prompt_tokens_details
    rendered: JsonValue = None
    if details is not None:
        rendered = {
            "text_tokens": details.text_tokens,
            "audio_tokens": details.audio_tokens,
            "image_tokens": details.image_tokens,
            "video_tokens": details.video_tokens,
        }
    return {
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "details": rendered,
    }


def embedding_value(answer: EmbeddingResponse) -> dict[str, JsonValue]:
    usage = answer.usage
    if usage is None:
        raise ValueError("the embedding reply carries no usage")
    return {
        "kind": "embedding",
        "model": answer.model,
        "data": len(DATA.validate_json(answer.model_dump_json())["data"]),
        "usage": usage_value(usage),
    }


def result_value(
    outcome: LoggingOutcome, operation: Operation
) -> dict[str, JsonValue]:
    match operation:
        case "predict" | "embed_content":
            answer = EmbeddingResponse.model_validate(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2298
                outcome["result"],  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L5066
                from_attributes=True,
            )
            return embedding_value(answer)
        case "predict_image":
            image = ImageResponse.model_validate(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2613
                outcome["result"],  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L5066
                from_attributes=True,
            )
            return {
                "kind": "image",
                "data": len(DATA.validate_json(image.model_dump_json())["data"]),
            }
        case "predict_audio":
            view = STANDARD.validate_python(outcome["result"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L5066
            return {"kind": "standard", "response": view["response"]}


def kwargs_value(kwargs: dict[str, JsonValue]) -> dict[str, JsonValue]:
    model = kwargs["model"]
    provider = kwargs["custom_llm_provider"]
    cost = kwargs["response_cost"]
    return {
        "model": model,
        "custom_llm_provider": provider,
        "response_cost": cost,
    }


def logging_value(logging_obj: Logging) -> dict[str, JsonValue]:
    logged = RecordedLogging.model_validate(logging_obj)
    details = logged.model_call_details
    return {
        "model": logged.model,
        "custom_llm_provider": logged.custom_llm_provider,
        "call_type": logged.call_type,
        "details": {
            "model": details.model,
            "custom_llm_provider": details.custom_llm_provider,
            "response_cost": details.response_cost,
        },
    }


def call_value(call: RecordedCall) -> dict[str, JsonValue]:
    return {
        "model": call.model,
        "custom_llm_provider": call.custom_llm_provider,
        "vertex_location": call.vertex_location,
        "response_model": call.response_model,
    }


def answer_value(call: Input, recorder: Recorder) -> JsonValue:
    logging_obj = make_logging()
    response = httpx.Response(call["status"], content=json.dumps(call["body"]).encode())
    kwargs: dict[str, JsonValue] = {}
    if call["operation"] == "embed_content":
        request_body = REQUEST_BODY.validate_python(call["request_body"])
        outcome = VertexHandler._handle_embed_content_response(  # noqa: SLF001  # type: ignore[misc]  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/vertex_passthrough_logging_handler.py#L496-L503
            httpx_response=response,
            logging_obj=logging_obj,
            url_route=call["url_route"],
            kwargs=kwargs,
            request_body=request_body,
        )
    else:
        outcome = VertexHandler._handle_predict_response(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/vertex_passthrough_logging_handler.py#L333-L338
            httpx_response=response,
            logging_obj=logging_obj,
            url_route=call["url_route"],
            kwargs=kwargs,
        )
    answers = cast("dict[str, JsonValue]", outcome["kwargs"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L5066
    return {
        "result": result_value(outcome, call["operation"]),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L5066
        "kwargs": kwargs_value(answers),
        "logging": logging_value(logging_obj),
        "cost": [call_value(recorded) for recorded in recorder.calls],
    }


def run(input_value: JsonValue) -> JsonValue:
    call = INPUT.validate_python(input_value)
    recorder = Recorder()
    litellm.completion_cost = recorder  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/cost_calculator.py#L1332
    try:
        answer = answer_value(call, recorder)
    except (AttributeError, KeyError, TypeError, ValueError):
        return {"failed": True}
    return answer
