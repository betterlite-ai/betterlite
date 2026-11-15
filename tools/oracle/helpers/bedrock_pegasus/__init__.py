import time
from collections.abc import Sequence
from functools import partial
from typing import (
    Final,
    Literal,
    NotRequired,
    Protocol,
    TypeAlias,
    TypedDict,
    runtime_checkable,
)

import httpx
from litellm.types.utils import Choices, LlmProviders, Message, ModelResponse
from litellm.utils import ProviderConfigManager
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

# The Pegasus transformation reads neither logging_obj nor encoding, so its view passes None for both.
@runtime_checkable
class Transform(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def transform_request(
        self,
        model: str,
        messages: Sequence[JsonValue],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        headers: dict[str, str],
    ) -> dict[str, JsonValue]: ...

    def transform_response(
        self,
        model: str,
        raw_response: httpx.Response,
        model_response: ModelResponse,
        logging_obj: None,
        request_data: dict[str, JsonValue],
        messages: Sequence[JsonValue],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        encoding: None,
        api_key: str | None = None,
        json_mode: bool | None = None,
    ) -> ModelResponse: ...


class TransformRecord(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    transform: Transform


PEGASUS_MODEL: Final = "twelvelabs.pegasus-1-2-v1:0"
REAL_TIME: Final = time.time
Role: TypeAlias = Literal["assistant", "user", "system", "tool", "function"]


def pegasus_config(model: str) -> Transform:
    config = ProviderConfigManager.get_provider_chat_config(
        model=model, provider=LlmProviders.BEDROCK
    )
    if config is None:
        raise ValueError("LiteLLM selects no Bedrock chat config")
    return TransformRecord.model_validate({"transform": config}).transform


class MessageSeed(TypedDict):
    role: Role
    content: str | None
    tool_calls: NotRequired[list[JsonValue] | None]


class ChoiceSeed(TypedDict):
    index: int
    finish_reason: str
    message: MessageSeed


class ResponseSeed(TypedDict):
    id: str
    choices: list[ChoiceSeed]


class MediaSourceCall(TypedDict):
    function: Literal["pegasus_media_source"]
    messages: list[JsonValue]
    optional_params: dict[str, JsonValue]


class PromptCall(TypedDict):
    function: Literal["text_video_prompt"]
    messages: list[JsonValue]
    optional_params: dict[str, JsonValue]


class MappingCall(TypedDict):
    function: Literal["response_format_json", "pegasus_options"]
    non_default: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    messages: list[JsonValue]


class ReplyCall(TypedDict):
    function: Literal[
        "segments_and_response", "usage_and_finish_reason", "pegasus_reply_errors"
    ]
    body: str
    status: int
    headers: list[list[str]]
    model: str
    messages: list[JsonValue]
    now: float
    response: ResponseSeed


Input: TypeAlias = MediaSourceCall | PromptCall | MappingCall | ReplyCall


class UsageView(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class ReplyMessageView(TypedDict):
    role: str
    content: NotRequired[str | None]


class ReplyChoiceView(TypedDict):
    index: int
    finish_reason: str
    message: ReplyMessageView


class ResponseView(TypedDict):
    id: str
    created: int
    model: str
    usage: UsageView
    choices: list[ReplyChoiceView]


RESPONSE: Final[TypeAdapter[ResponseView]] = TypeAdapter(ResponseView)


def build_model_response(seed: ResponseSeed) -> ModelResponse:
    choices: list[Choices] = []
    for choice in seed["choices"]:
        picked = choice["message"]
        message = Message(content=picked["content"], role=picked["role"])
        if picked.get("tool_calls") is not None:
            message.tool_calls = []
        choices.append(
            Choices(
                index=choice["index"],
                message=message,
                finish_reason=choice["finish_reason"],
            )
        )
    return ModelResponse(id=seed["id"], choices=choices, created=0, model="")


def request_params(
    messages: Sequence[JsonValue], optional_params: dict[str, JsonValue]
) -> dict[str, JsonValue]:
    return pegasus_config(PEGASUS_MODEL).transform_request(
        model=PEGASUS_MODEL,
        messages=messages,
        optional_params=optional_params,
        litellm_params={},
        headers={},
    )


def mapping_view(call: MappingCall) -> JsonValue:
    config = pegasus_config(PEGASUS_MODEL)
    supported: list[JsonValue] = []
    supported.extend(config.get_supported_openai_params(model=PEGASUS_MODEL))
    mapped = config.map_openai_params(
        non_default_params=call["non_default"],
        optional_params=call["optional_params"],
        model=PEGASUS_MODEL,
        drop_params=False,
    )
    result: JsonValue = {
        "supported": supported,
        "mapped": mapped,
        "request": request_params(call["messages"], mapped),
    }
    return result


def reply_view(call: ReplyCall) -> JsonValue:
    raw_response = httpx.Response(
        call["status"],
        content=call["body"].encode(),
        headers=[(name, value) for name, value in call["headers"]],
    )
    time.time = partial(float, call["now"])
    try:
        transformed = pegasus_config(PEGASUS_MODEL).transform_response(
            model=call["model"],
            raw_response=raw_response,
            model_response=build_model_response(call["response"]),
            logging_obj=None,
            request_data={},
            messages=call["messages"],
            optional_params={},
            litellm_params={},
            encoding=None,
            api_key=None,
            json_mode=False,
        )
    finally:
        time.time = REAL_TIME
    view = RESPONSE.validate_json(transformed.model_dump_json())
    choices: list[JsonValue] = []
    for choice in view["choices"]:
        entry: JsonValue = {
            "index": choice["index"],
            "finish_reason": choice["finish_reason"],
            "role": choice["message"]["role"],
            "content": choice["message"].get("content"),
        }
        choices.append(entry)
    usage: JsonValue = {
        "prompt_tokens": view["usage"]["prompt_tokens"],
        "completion_tokens": view["usage"]["completion_tokens"],
        "total_tokens": view["usage"]["total_tokens"],
    }
    result: JsonValue = {
        "id": view["id"],
        "created": view["created"],
        "model": view["model"],
        "usage": usage,
        "choices": choices,
    }
    return result


def run(input_value: Input) -> JsonValue:
    match input_value["function"]:
        case "pegasus_media_source" | "text_video_prompt":
            answer: JsonValue = {
                "request": request_params(
                    input_value["messages"], input_value["optional_params"]
                )
            }
            return answer
        case "response_format_json" | "pegasus_options":
            return mapping_view(input_value)
        case "segments_and_response" | "usage_and_finish_reason" | "pegasus_reply_errors":
            return reply_view(input_value)
