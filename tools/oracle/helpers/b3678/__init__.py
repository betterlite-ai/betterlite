import asyncio
from typing import Final, Literal, TypeAlias, TypedDict

from litellm.llms.bedrock.passthrough.guardrail_translation.handler import (
    BedrockPassthroughGuardrailHandler,
)
from litellm.proxy._types import UserAPIKeyAuth
from pydantic import JsonValue, TypeAdapter
from typing_extensions import NotRequired

from b3678 import frames, scripted

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
HANDLER: Final = BedrockPassthroughGuardrailHandler()


class RoutingInput(TypedDict):
    function: Literal["routing"]
    endpoint: str
    content_type: str


class InputTextsInput(TypedDict):
    function: Literal["input_texts"]
    endpoint: str
    data: dict[str, JsonValue]
    skip_system: bool
    skip_tool: bool
    returned_texts: NotRequired[list[str]]


class OutputTextsInput(TypedDict):
    function: Literal["output_texts"]
    endpoint: str
    response: dict[str, JsonValue]
    request_data: dict[str, JsonValue] | None
    returned_texts: NotRequired[list[str]]


class FrameInput(TypedDict):
    event_type: str
    payload: JsonValue


class StreamInput(TypedDict):
    function: Literal["stream"]
    frames: list[FrameInput]
    replace: dict[str, str]
    trailing_hex: str


Input: TypeAlias = RoutingInput | InputTextsInput | OutputTextsInput | StreamInput


def routing(input_value: RoutingInput) -> JsonValue:
    return JSON.validate_python(
        {
            "is_converse": HANDLER.event_stream_endpoint_is_de_anonymizable(input_value["endpoint"]),
            "is_event_stream": HANDLER.is_event_stream_content_type(input_value["content_type"]),
            "media_type": HANDLER.event_stream_media_type(),
        }
    )


def input_texts(input_value: InputTextsInput) -> JsonValue:
    guardrail = scripted.ScriptedGuardrail(
        input_value.get("returned_texts"), input_value["skip_system"], input_value["skip_tool"]
    )
    data = dict(input_value["data"])
    data["endpoint"] = input_value["endpoint"]
    result = asyncio.run(HANDLER.process_input_messages(data=data, guardrail_to_apply=guardrail))
    return JSON.validate_python({"texts": guardrail.texts, "model": guardrail.model, "data": result})


def output_texts(input_value: OutputTextsInput) -> JsonValue:
    guardrail = scripted.ScriptedGuardrail(input_value.get("returned_texts"), False, False)
    request_data = input_value["request_data"]
    if request_data is not None:
        request_data = {**request_data, "endpoint": input_value["endpoint"]}
    result = asyncio.run(
        HANDLER.process_output_response(
            response=input_value["response"],
            guardrail_to_apply=guardrail,
            request_data=request_data,
        )
    )
    return JSON.validate_python({"texts": guardrail.texts, "model": guardrail.model, "data": result})


def stream(input_value: StreamInput) -> JsonValue:
    body = frames.encode_stream(input_value["frames"], input_value["trailing_hex"])
    hook = scripted.ScriptedHook(input_value["replace"])
    rewritten = asyncio.run(
        HANDLER.de_anonymize_event_stream(
            body_bytes=body,
            proxy_logging_obj=hook,
            user_api_key_dict=UserAPIKeyAuth(),
            data={},
        )
    )
    return JSON.validate_python(frames.decode_stream(rewritten))


def run(input_value: Input) -> JsonValue:
    match input_value["function"]:
        case "routing":
            return routing(input_value)
        case "input_texts":
            return input_texts(input_value)
        case "output_texts":
            return output_texts(input_value)
        case "stream":
            return stream(input_value)
