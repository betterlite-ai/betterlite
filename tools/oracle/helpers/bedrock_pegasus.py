import time
from functools import partial
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.bedrock.chat.invoke_transformations import (
    amazon_twelvelabs_pegasus_transformation as pegasus_module,
)
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import Choices, Message, ModelResponse
from pydantic import JsonValue, TypeAdapter

REAL_TIME: Final = time.time
Role: TypeAlias = Literal["assistant", "user", "system", "tool", "function"]
PEGASUS = pegasus_module.AmazonTwelveLabsPegasusConfig()


class Mapper(Protocol):
    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...


# LiteLLM leaves the optional params bare, so this typed view is the public entry to the mapping
MAPPER: Mapper = PEGASUS


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
    messages: list[AllMessageValues]
    optional_params: dict[str, JsonValue]


class PromptCall(TypedDict):
    function: Literal["text_video_prompt"]
    messages: list[AllMessageValues]
    optional_params: dict[str, JsonValue]


class MappingCall(TypedDict):
    function: Literal["response_format_json", "pegasus_options"]
    non_default: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    messages: list[AllMessageValues]


class ReplyCall(TypedDict):
    function: Literal[
        "segments_and_response", "usage_and_finish_reason", "pegasus_reply_errors"
    ]
    body: str
    status: int
    headers: list[list[str]]
    model: str
    messages: list[AllMessageValues]
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
    messages: list[AllMessageValues], optional_params: dict[str, JsonValue]
) -> dict[str, JsonValue]:
    return PEGASUS.transform_request(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/amazon_twelvelabs_pegasus_transformation.py#L103-L130
        model="",
        messages=messages,
        optional_params=optional_params,
        litellm_params={},
        headers={},
    )


def request_view(
    messages: list[AllMessageValues], optional_params: dict[str, JsonValue]
) -> JsonValue:
    result: JsonValue = {"request": request_params(messages, optional_params)}
    return result


def mapping_view(call: MappingCall) -> JsonValue:
    supported: list[JsonValue] = []
    supported.extend(PEGASUS.get_supported_openai_params(model=""))
    mapped = MAPPER.map_openai_params(
        non_default_params=call["non_default"],
        optional_params=call["optional_params"],
        model="",
        drop_params=False,
    )
    result: JsonValue = {
        "supported": supported,
        "mapped": mapped,
        "request": request_params(call["messages"], mapped),
    }
    return result


def reply_view(call: ReplyCall) -> JsonValue:
    response = httpx.Response(
        call["status"],
        content=call["body"].encode(),
        headers=[(name, value) for name, value in call["headers"]],
    )
    time.time = partial(float, call["now"])
    try:
        transformed = PEGASUS.transform_response(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/amazon_twelvelabs_pegasus_transformation.py#L182-L270
            model=call["model"],
            raw_response=response,
            model_response=build_model_response(call["response"]),
            logging_obj=Logging.__new__(Logging),
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
            return request_view(
                input_value["messages"], input_value["optional_params"]
            )
        case "response_format_json" | "pegasus_options":
            return mapping_view(input_value)
        case "segments_and_response" | "usage_and_finish_reason" | "pegasus_reply_errors":
            return reply_view(input_value)
