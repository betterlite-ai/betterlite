import os
import uuid
from collections.abc import Callable
from datetime import UTC, datetime
from typing import NotRequired, Protocol, TypeAlias, TypedDict, cast
from unittest.mock import patch

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.llms.fireworks_ai.chat.transformation import FireworksAIConfig
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import ModelResponse
from pydantic import BaseModel, Field, JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
FIXED_MODEL = "accounts/fireworks/models/test-model"
FIXED_UUID = uuid.UUID(int=1)
FIXED_TIME = 1_700_000_000
MANAGED_ENV = (
    "FIREWORKS_API_KEY",
    "FIREWORKS_AI_API_KEY",
    "FIREWORKSAI_API_KEY",
    "FIREWORKS_AI_TOKEN",
    "FIREWORKS_API_BASE",
    "FIREWORKS_ACCOUNT_ID",
)


class CatalogModel(TypedDict):
    key: str
    model_info: dict[str, JsonValue]


class HeaderCall(TypedDict):
    headers: dict[str, str]
    api_key: str | None
    litellm_params: dict[str, JsonValue]
    environment: dict[str, str]


class KeyCall(TypedDict):
    api_key: str | None
    environment: dict[str, str]


class RequestCall(TypedDict):
    model: str
    messages: list[AllMessageValues]
    optional_params: dict[str, JsonValue]


class ReplyCall(TypedDict):
    status: int
    headers: dict[str, str]
    body: str
    optional_params: dict[str, JsonValue]


class ChunkCall(TypedDict):
    chunk: dict[str, JsonValue]


class ModelCall(TypedDict):
    api_base: str | None
    api_key: str | None
    environment: dict[str, str]
    status: int
    body: str


class Input(TypedDict):
    catalog: list[CatalogModel]
    headers: list[HeaderCall]
    keys: list[KeyCall]
    requests: list[RequestCall]
    replies: list[ReplyCall]
    chunks: list[ChunkCall]
    models: list[ModelCall]


class UsageDump(TypedDict, total=False):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class ToolFunctionDump(TypedDict, total=False):
    name: str
    arguments: str


class ToolCallDump(TypedDict, total=False):
    id: str
    type: str
    function: ToolFunctionDump


class MessageDump(TypedDict, total=False):
    content: JsonValue
    tool_calls: list[ToolCallDump] | None


class ChoiceDump(TypedDict, total=False):
    index: int
    finish_reason: str | None
    message: MessageDump
    provider_specific_fields: dict[str, JsonValue] | None


class ReplyDump(TypedDict):
    choices: list[ChoiceDump]
    id: NotRequired[str | None]
    object: NotRequired[str | None]
    created: NotRequired[int | None]
    model: NotRequired[str | None]
    usage: NotRequired[UsageDump | None]


class ChunkDeltaDump(TypedDict, total=False):
    role: str | None
    content: str | None


class ChunkChoiceDump(TypedDict, total=False):
    index: int
    finish_reason: str | None
    delta: ChunkDeltaDump


class ChunkDump(TypedDict):
    choices: list[ChunkChoiceDump]
    id: NotRequired[str | None]
    object: NotRequired[str | None]
    created: NotRequired[int | None]
    model: NotRequired[str | None]
    provider_specific_fields: NotRequired[dict[str, JsonValue] | None]


class HiddenCarrier(BaseModel):
    # the key set depends on the payload, so the hidden params stay open JSON
    params: dict[str, JsonValue] = Field(alias="_hidden_params")


REPLY: TypeAdapter[ReplyDump] = TypeAdapter(ReplyDump)
CHUNK: TypeAdapter[ChunkDump] = TypeAdapter(ChunkDump)

ValidateEnvironment: TypeAlias = Callable[
    [
        dict[str, str],
        str,
        list[AllMessageValues],
        dict[str, JsonValue],
        dict[str, JsonValue],
        str | None,
        str | None,
    ],
    dict[str, str],
]
TransformRequest: TypeAlias = Callable[
    [str, list[AllMessageValues], dict[str, JsonValue], dict[str, JsonValue], dict[str, str]],
    dict[str, JsonValue],
]
GetModels: TypeAlias = Callable[[str | None, str | None], list[str]]


class FireworksChat(Protocol):
    @property
    def validate_environment(self) -> ValidateEnvironment: ...

    @property
    def transform_request(self) -> TransformRequest: ...

    @property
    def get_models(self) -> GetModels: ...


class RecordingTransport(httpx.MockTransport):
    def __init__(self, status: int, body: str) -> None:
        super().__init__(self.answer)
        self.status = status
        self.body = body
        self.url: str | None = None
        self.authorization: str | None = None

    def answer(self, request: httpx.Request) -> httpx.Response:
        self.url = str(request.url)
        self.authorization = request.headers.get("authorization")
        return httpx.Response(status_code=self.status, content=self.body.encode())


def fixed_uuid4() -> uuid.UUID:
    return FIXED_UUID


def fixed_time() -> float:
    return float(FIXED_TIME)


def activate(environment: dict[str, str]) -> None:
    for name in MANAGED_ENV:
        os.environ.pop(name, None)
    for name, value in environment.items():
        os.environ[name] = value


def raw_response(call: ReplyCall) -> httpx.Response:
    request = httpx.Request("POST", "https://api.fireworks.ai/inference/v1/chat/completions")
    # content= would make httpx add its own Content-Length, and the case lists the upstream headers
    return httpx.Response(
        status_code=call["status"],
        headers=call["headers"],
        stream=httpx.ByteStream(call["body"].encode()),
        request=request,
    )


def logging_object() -> Logging:
    return Logging(
        model=FIXED_MODEL,
        messages=None,
        stream=False,
        call_type="completion",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
    )


def header_value(chat: FireworksChat, call: HeaderCall) -> JsonValue:
    activate(call["environment"])
    headers = chat.validate_environment(
        dict(call["headers"]),
        FIXED_MODEL,
        [],
        {},
        call["litellm_params"],
        call["api_key"],
        None,
    )
    return JSON.validate_python(headers)


def key_value(call: KeyCall) -> JsonValue:
    activate(call["environment"])
    return FireworksAIConfig.get_api_key(call["api_key"])


def request_value(chat: FireworksChat, call: RequestCall) -> JsonValue:
    return chat.transform_request(
        call["model"], call["messages"], call["optional_params"], {}, {}
    )


def usage_value(usage: UsageDump | None) -> JsonValue:
    if usage is None:
        return None
    return {
        "prompt_tokens": usage.get("prompt_tokens"),
        "completion_tokens": usage.get("completion_tokens"),
        "total_tokens": usage.get("total_tokens"),
    }


def tool_call_value(call: ToolCallDump) -> JsonValue:
    function = call.get("function")
    name = None if function is None else function.get("name")
    arguments = None if function is None else function.get("arguments")
    return {
        "id": call.get("id"),
        "type": call.get("type"),
        "function": {"name": name, "arguments": arguments},
    }


def tool_calls_value(calls: list[ToolCallDump] | None) -> JsonValue:
    if calls is None:
        return None
    return [tool_call_value(call) for call in calls]


def choice_value(choice: ChoiceDump) -> JsonValue:
    message = choice.get("message")
    native = choice.get("provider_specific_fields")
    return {
        "index": choice.get("index"),
        "finish_reason": choice.get("finish_reason"),
        "content": None if message is None else message.get("content"),
        "tool_calls": None if message is None else tool_calls_value(message.get("tool_calls")),
        "native_finish_reason": None if native is None else native.get("native_finish_reason"),
    }


def reply_value(config: FireworksAIConfig, call: ReplyCall) -> JsonValue:
    response = config.transform_response(
        model=FIXED_MODEL,
        raw_response=raw_response(call),
        model_response=ModelResponse(),
        logging_obj=logging_object(),
        request_data={},
        messages=[],
        optional_params=call["optional_params"],
        litellm_params={},
        encoding=None,
        api_key=None,
        json_mode=None,
    )
    dump = REPLY.validate_json(response.model_dump_json())
    hidden = HiddenCarrier.model_validate(response, from_attributes=True).params
    return {
        "id": dump.get("id"),
        "object": dump.get("object"),
        "created": dump.get("created"),
        "model": dump.get("model"),
        "choices": [choice_value(choice) for choice in dump["choices"]],
        "usage": usage_value(dump.get("usage")),
        "hidden": hidden,
    }


def chunk_choice_value(choice: ChunkChoiceDump) -> JsonValue:
    delta = choice.get("delta")
    role = None if delta is None else delta.get("role")
    content = None if delta is None else delta.get("content")
    return {
        "index": choice.get("index"),
        "finish_reason": choice.get("finish_reason"),
        "role": role,
        "content": content,
    }


def chunk_value(config: FireworksAIConfig, call: ChunkCall) -> JsonValue:
    handler = config.get_model_response_iterator(iter(()), sync_stream=True, json_mode=False)
    parsed = handler.chunk_parser(call["chunk"])
    dump = CHUNK.validate_json(parsed.model_dump_json())
    return {
        "id": dump.get("id"),
        "object": dump.get("object"),
        "created": dump.get("created"),
        "model": dump.get("model"),
        "choices": [chunk_choice_value(choice) for choice in dump["choices"]],
        "provider_specific_fields": dump.get("provider_specific_fields"),
    }


def model_value(chat: FireworksChat, call: ModelCall) -> JsonValue:
    activate(call["environment"])
    recorder = RecordingTransport(call["status"], call["body"])
    litellm.module_level_client = HTTPHandler(client=httpx.Client(transport=recorder))
    models = chat.get_models(call["api_key"], call["api_base"])
    return {
        "models": JSON.validate_python(models),
        "url": recorder.url,
        "authorization": recorder.authorization,
    }


def run(input_value: Input) -> JsonValue:
    litellm.model_cost = {entry["key"]: entry["model_info"] for entry in input_value["catalog"]}
    config = FireworksAIConfig()
    chat: FireworksChat = cast(FireworksChat, config)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/fireworks_ai/chat/transformation.py#L653-L665
    answer: dict[str, JsonValue] = {}
    # LiteLLM mints ids and timestamps with fastuuid and time, and the cases fix both
    with (
        patch("litellm.types.utils.uuid.uuid4", fixed_uuid4),
        patch("time.time", fixed_time),
    ):
        answer["headers"] = [header_value(chat, call) for call in input_value["headers"]]
        answer["keys"] = [key_value(call) for call in input_value["keys"]]
        answer["requests"] = [request_value(chat, call) for call in input_value["requests"]]
        answer["replies"] = [reply_value(config, call) for call in input_value["replies"]]
        answer["chunks"] = [chunk_value(config, call) for call in input_value["chunks"]]
        answer["models"] = [model_value(chat, call) for call in input_value["models"]]
    return answer
