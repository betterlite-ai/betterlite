import os
from datetime import UTC, datetime
from typing import Literal, Protocol, TypeAlias, TypedDict
from unittest.mock import patch

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.llms.hosted_vllm.chat.transformation import HostedVLLMChatConfig
from litellm.llms.hosted_vllm.embedding.transformation import HostedVLLMEmbeddingConfig
from litellm.types.llms.openai import AllEmbeddingInputValues, AllMessageValues
from litellm.types.utils import CallTypes, EmbeddingResponse
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

OBJECT = TypeAdapter(dict[str, JsonValue])
CHAT_REPLY: dict[str, JsonValue] = {
    "choices": [
        {
            "index": 0,
            "finish_reason": "stop",
            "message": {"role": "assistant", "content": "hi"},
        }
    ],
    "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
}


class ChatConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...


class EmbeddingConfig(Protocol):
    def validate_environment(
        self,
        _headers: dict[str, str],
        _model: str,
        _messages: list[AllMessageValues],
        _optional_params: dict[str, JsonValue],
        _litellm_params: dict[str, JsonValue],
        _api_key: str | None = None,
        _api_base: str | None = None,
        /,
    ) -> dict[str, JsonValue]: ...

    def get_complete_url(
        self,
        _api_base: str | None,
        _api_key: str | None,
        _model: str,
        _optional_params: dict[str, JsonValue],
        _litellm_params: dict[str, JsonValue],
        /,
        *,
        _stream: bool | None = None,
    ) -> str: ...

    def transform_embedding_request(
        self,
        model: str,
        _input: AllEmbeddingInputValues,
        optional_params: dict[str, JsonValue],
        headers: dict[str, str],
        /,
    ) -> dict[str, JsonValue]: ...

    def transform_embedding_response(
        self,
        _model: str,
        _raw_response: httpx.Response,
        _model_response: EmbeddingResponse,
        _logging_obj: Logging,
        _api_key: str | None,
        _request_data: dict[str, JsonValue],
        _optional_params: dict[str, JsonValue],
        _litellm_params: dict[str, JsonValue],
        /,
    ) -> EmbeddingResponse: ...


class SupportedCall(TypedDict):
    model: str


class MapCall(TypedDict):
    model: str
    params: dict[str, JsonValue]
    preset: dict[str, JsonValue]


class MessagesCall(TypedDict):
    model: str
    messages: list[dict[str, JsonValue]]


class HeadersCall(TypedDict):
    headers: dict[str, str]
    api_key: str | None
    env: dict[str, str]


class UrlCall(TypedDict):
    api_base: str | None
    env: dict[str, str]


class RequestCall(TypedDict):
    model: str
    input: AllEmbeddingInputValues
    optional_params: dict[str, JsonValue]


class ResponseCall(TypedDict):
    body: str


class ChatSupportedInput(TypedDict):
    function: Literal["chat_supported"]
    calls: list[SupportedCall]


class ChatMapInput(TypedDict):
    function: Literal["chat_map"]
    calls: list[MapCall]


class ChatMessagesInput(TypedDict):
    function: Literal["chat_messages"]
    calls: list[MessagesCall]


class EmbeddingHeadersInput(TypedDict):
    function: Literal["embedding_headers"]
    calls: list[HeadersCall]


class EmbeddingUrlInput(TypedDict):
    function: Literal["embedding_url"]
    calls: list[UrlCall]


class EmbeddingRequestInput(TypedDict):
    function: Literal["embedding_request"]
    calls: list[RequestCall]


class EmbeddingResponseInput(TypedDict):
    function: Literal["embedding_response"]
    calls: list[ResponseCall]


Input: TypeAlias = (
    ChatSupportedInput
    | ChatMapInput
    | ChatMessagesInput
    | EmbeddingHeadersInput
    | EmbeddingUrlInput
    | EmbeddingRequestInput
    | EmbeddingResponseInput
)
INPUT = TypeAdapter(Input)


class UsageCounters(BaseModel):
    model_config = ConfigDict(extra="ignore")
    prompt_tokens: int = 0
    completion_tokens: int = 0
    total_tokens: int = 0


class EmbeddingReply(BaseModel):
    model_config = ConfigDict(extra="ignore")
    model: str
    object: str
    data: list[JsonValue]
    usage: JsonValue = None


REPLY = TypeAdapter(EmbeddingReply)
USAGE = TypeAdapter(UsageCounters | None)


class Recorder:
    def __init__(self) -> None:
        self.body: dict[str, JsonValue] = {}

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.body = OBJECT.validate_json(request.content)
        return httpx.Response(200, json=CHAT_REPLY)


CHAT: ChatConfig = HostedVLLMChatConfig()
EMBEDDING: EmbeddingConfig = HostedVLLMEmbeddingConfig()


def chat_supported_value(call: SupportedCall) -> JsonValue:
    params: list[JsonValue] = [*CHAT.get_supported_openai_params(call["model"])]
    return params


def chat_map_value(call: MapCall) -> JsonValue:
    optional: dict[str, JsonValue] = dict(call["preset"])
    mapped: dict[str, JsonValue] = CHAT.map_openai_params(
        dict(call["params"]), optional, call["model"], drop_params=False
    )
    return mapped


def chat_messages_value(call: MessagesCall) -> JsonValue:
    recorder = Recorder()
    litellm.completion(
        model=f"hosted_vllm/{call['model']}",
        messages=call["messages"],
        api_base="http://hosted-vllm.invalid",
        api_key="k",
        client=HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(recorder.handle))),
    )
    return recorder.body["messages"]


def embedding_headers_value(call: HeadersCall) -> JsonValue:
    headers: dict[str, JsonValue]
    with patch.dict(os.environ, call["env"], clear=True):
        headers = EMBEDDING.validate_environment(
            dict(call["headers"]),
            "m",
            [],
            {},
            {},
            call["api_key"],
        )
    return headers


def embedding_url_value(call: UrlCall) -> JsonValue:
    with patch.dict(os.environ, call["env"], clear=True):
        return EMBEDDING.get_complete_url(call["api_base"], None, "m", {}, {})


def embedding_request_value(call: RequestCall) -> JsonValue:
    headers: dict[str, str] = {}
    body: dict[str, JsonValue] = EMBEDDING.transform_embedding_request(
        call["model"], call["input"], call["optional_params"], headers
    )
    return body


def logging_object() -> Logging:
    return Logging(
        model="m",
        messages=None,
        stream=False,
        call_type=CallTypes.embedding,
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="b8150",
        function_id="b8150",
    )


def reply_value(response: EmbeddingResponse) -> JsonValue:
    parsed = REPLY.validate_json(response.model_dump_json())
    usage = USAGE.validate_python(parsed.usage)
    counts: dict[str, JsonValue] = {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}
    if usage is not None:
        counts["prompt_tokens"] = usage.prompt_tokens
        counts["completion_tokens"] = usage.completion_tokens
        counts["total_tokens"] = usage.total_tokens
    value: dict[str, JsonValue] = {
        "model": parsed.model,
        "object": parsed.object,
        "data": parsed.data,
        "usage": counts,
    }
    return value


def embedding_response_value(call: ResponseCall) -> JsonValue:
    raw = httpx.Response(
        200,
        content=call["body"].encode(),
        request=httpx.Request("POST", "https://host"),
    )
    response = EMBEDDING.transform_embedding_response(
        "m", raw, EmbeddingResponse(), logging_object(), None, {}, {}, {}
    )
    return reply_value(response)


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    parsed = INPUT.validate_python(input_value)
    result: JsonValue
    if parsed["function"] == "chat_supported":
        result = [chat_supported_value(call) for call in parsed["calls"]]
    elif parsed["function"] == "chat_map":
        result = [chat_map_value(call) for call in parsed["calls"]]
    elif parsed["function"] == "chat_messages":
        result = [chat_messages_value(call) for call in parsed["calls"]]
    elif parsed["function"] == "embedding_headers":
        result = [embedding_headers_value(call) for call in parsed["calls"]]
    elif parsed["function"] == "embedding_url":
        result = [embedding_url_value(call) for call in parsed["calls"]]
    elif parsed["function"] == "embedding_request":
        result = [embedding_request_value(call) for call in parsed["calls"]]
    else:
        result = [embedding_response_value(call) for call in parsed["calls"]]
    return result
