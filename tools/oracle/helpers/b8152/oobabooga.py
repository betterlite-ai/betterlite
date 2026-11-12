from typing import Protocol, TypedDict, cast

import httpx
import litellm
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.llms.oobabooga.chat import oobabooga
from litellm.llms.oobabooga.chat.transformation import OobaboogaConfig
from litellm.llms.oobabooga.common_utils import OobaboogaError
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import EmbeddingResponse, ModelResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

HEADER_NAMES = ("accept", "content-type", "authorization")
CHAT_REPLY = (
    '{"choices": [{"message": {"content": "ok"}}], '
    '"usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}}'
)
NO_PARAMS: dict[str, JsonValue] = {}
NO_MESSAGES: list[AllMessageValues] = []
CLIENT_KEY = "httpx_client"
HOUR = 3600


class RequestCall(TypedDict):
    model: str
    api_base: str | None
    api_key: str | None
    messages: list[dict[str, JsonValue]]
    optional_params: dict[str, JsonValue]


class EmbeddingCall(TypedDict):
    model: str
    api_base: str | None
    api_key: str | None
    input: list[str]
    optional_params: dict[str, JsonValue]
    reply: str


class ReplyCall(TypedDict):
    model: str
    status: int
    body: str


class RecordingTransport:
    def __init__(self, body: str) -> None:
        self.body = body
        self.requests: list[httpx.Request] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        return httpx.Response(200, content=self.body.encode(), request=request)

    def request_value(self) -> dict[str, JsonValue]:
        request = self.requests[0]
        headers: dict[str, JsonValue] = {
            name: request.headers[name] for name in HEADER_NAMES if name in request.headers
        }
        return {
            "url": str(request.url),
            "headers": headers,
            "body": TypeAdapter(JsonValue).validate_json(request.content),
        }


class LoggingHooks(Protocol):
    def pre_call(self, **params: JsonValue) -> None: ...

    def post_call(self, **params: JsonValue) -> None: ...


class NoLogging(LoggingHooks):
    @override
    def pre_call(self, **params: JsonValue) -> None:
        del params

    @override
    def post_call(self, **params: JsonValue) -> None:
        del params


def request_value(call: RequestCall) -> JsonValue:
    transport = RecordingTransport(CHAT_REPLY)
    client = HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(transport.handle)))
    litellm.in_memory_llm_clients_cache.set_cache(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/llm_caching_handler.py#L58
        key=CLIENT_KEY, value=client, ttl=HOUR
    )
    optional: dict[str, JsonValue] = dict(call["optional_params"])
    try:
        oobabooga.completion(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/oobabooga/chat/oobabooga.py#L17-L33
            model=call["model"],
            messages=call["messages"],
            api_base=call["api_base"],
            model_response=ModelResponse(),
            print_verbose=print,
            encoding=None,
            api_key=call["api_key"],
            logging_obj=NoLogging(),
            optional_params=optional,
            litellm_params=NO_PARAMS,
        )
    except OobaboogaError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/oobabooga/common_utils.py#L6
        return {"ok": False, "status": error.status_code, "message": error.message}
    return {"ok": True, "request": transport.request_value()}


def embedding_value(call: EmbeddingCall) -> JsonValue:
    transport = RecordingTransport(call["reply"])
    litellm.module_level_client = HTTPHandler(
        client=httpx.Client(transport=httpx.MockTransport(transport.handle))
    )
    optional: dict[str, JsonValue] = dict(call["optional_params"])
    result = EmbeddingResponse()
    try:
        oobabooga.embedding(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/oobabooga/chat/oobabooga.py#L113-L122
            model=call["model"],
            input=call["input"],
            model_response=result,
            api_key=call["api_key"],
            api_base=call["api_base"],
            logging_obj=NoLogging(),  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/oobabooga/chat/oobabooga.py#L117
            optional_params=optional,
        )
    except OobaboogaError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/oobabooga/common_utils.py#L6
        return {"ok": False, "status": error.status_code, "message": error.message}
    usage = cast("dict[str, JsonValue]", result.usage.model_dump())  # type: ignore[union-attr]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/oobabooga/chat/oobabooga.py#L172-L176
    return {
        "ok": True,
        "request": transport.request_value(),
        "model": result.model,
        "data": cast("list[JsonValue]", result.data),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/oobabooga/chat/oobabooga.py#L161-L167
        "prompt_tokens": usage["prompt_tokens"],
        "total_tokens": usage["total_tokens"],
    }


def reply_value(config: OobaboogaConfig, call: ReplyCall) -> JsonValue:
    response = httpx.Response(call["status"], content=call["body"].encode())
    try:
        result = config.transform_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/oobabooga/chat/transformation.py#L30-L42
            model=call["model"],
            raw_response=response,
            model_response=ModelResponse(),
            logging_obj=NoLogging(),  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/oobabooga/chat/transformation.py#L32
            request_data=NO_PARAMS,
            messages=NO_MESSAGES,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/oobabooga/chat/transformation.py#L34
            optional_params=NO_PARAMS,
            litellm_params=NO_PARAMS,
            encoding=None,
        )
    except OobaboogaError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/oobabooga/common_utils.py#L6
        return {"ok": False, "status": error.status_code, "message": error.message}
    usage = cast("dict[str, JsonValue]", result.usage.model_dump())  # type: ignore[attr-defined,misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/oobabooga/chat/transformation.py#L66-L72
    return {
        "ok": True,
        "model": result.model,
        "content": result.choices[0].message.content,  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/oobabooga/chat/transformation.py#L56
        "usage": {
            "prompt_tokens": usage["prompt_tokens"],
            "completion_tokens": usage["completion_tokens"],
            "total_tokens": usage["total_tokens"],
        },
    }
