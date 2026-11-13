from collections.abc import Callable
from datetime import UTC, datetime
from typing import Final, Protocol, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.sambanova.chat import SambanovaConfig
from litellm.llms.sambanova.common_utils import SambaNovaError
from litellm.llms.sambanova.embedding.transformation import SambaNovaEmbeddingConfig
from litellm.types.llms.openai import AllEmbeddingInputValues, AllMessageValues
from litellm.types.utils import EmbeddingResponse
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)
INPUTS: Final = TypeAdapter(AllEmbeddingInputValues)

MODEL: Final = "sambanova/embedding"
STARTED: Final = datetime(2020, 1, 1, tzinfo=UTC)

ValidateEnvironment = Callable[
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
TransformRequest = Callable[
    [str, AllEmbeddingInputValues, dict[str, JsonValue], dict[str, str]],
    dict[str, JsonValue],
]
TransformResponse = Callable[
    [
        str,
        httpx.Response,
        EmbeddingResponse,
        Logging,
        str | None,
        dict[str, JsonValue],
        dict[str, JsonValue],
        dict[str, JsonValue],
    ],
    EmbeddingResponse,
]


class ChatConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[JsonValue]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...


class EmbeddingConfig(Protocol):
    validate_environment: ValidateEnvironment
    transform_embedding_request: TransformRequest
    transform_embedding_response: TransformResponse

    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
    ) -> str: ...

    def get_error_class(
        self, error_message: str, status_code: int, headers: dict[str, str]
    ) -> BaseLLMException: ...


class SupportedCall(TypedDict):
    model: str
    function_calling: bool


class ParamsCall(TypedDict):
    model: str
    function_calling: bool
    requested: dict[str, JsonValue]
    existing: dict[str, JsonValue]


class UrlCall(TypedDict):
    api_base: str | None


class HeadersCall(TypedDict):
    api_key: str | None
    headers: dict[str, str]


class RequestCall(TypedDict):
    model: str
    input: JsonValue
    optional: dict[str, JsonValue]


class ReplyCall(TypedDict):
    model: str
    body: str
    status: int
    headers: dict[str, str]


class ErrorCall(TypedDict):
    message: str
    status: int
    headers: dict[str, str]


class UsageShape(TypedDict):
    prompt_tokens: int
    total_tokens: int


class ReplyShape(TypedDict):
    model: str | None
    object: str | None
    data: list[JsonValue] | None
    usage: UsageShape


class Input(TypedDict):
    chat_supported: list[SupportedCall]
    chat_params: list[ParamsCall]
    embedding_urls: list[UrlCall]
    embedding_headers: list[HeadersCall]
    embedding_requests: list[RequestCall]
    embedding_replies: list[ReplyCall]
    embedding_errors: list[ErrorCall]


REPLY: Final = TypeAdapter(ReplyShape)


def supported(config: ChatConfig, call: SupportedCall) -> JsonValue:
    params = config.get_supported_openai_params(call["model"])
    answer: dict[str, JsonValue] = {"model": call["model"], "supported": params}
    return JSON.validate_python(answer)


def params(config: ChatConfig, call: ParamsCall) -> JsonValue:
    mapped = config.map_openai_params(
        call["requested"], dict(call["existing"]), call["model"], drop_params=False
    )
    answer: dict[str, JsonValue] = {"model": call["model"], "mapped": mapped}
    return JSON.validate_python(answer)


def url(config: EmbeddingConfig, call: UrlCall) -> JsonValue:
    try:
        resolved: str | None = config.get_complete_url(call["api_base"], None, MODEL, {}, {})
    except ValueError:
        resolved = None
    answer: dict[str, JsonValue] = {"url": resolved}
    return JSON.validate_python(answer)


def headers(config: EmbeddingConfig, call: HeadersCall) -> JsonValue:
    validated = config.validate_environment(
        dict(call["headers"]), MODEL, [], {}, {}, call["api_key"], None
    )
    answer: dict[str, JsonValue] = {"headers": JSON.validate_python(validated)}
    return JSON.validate_python(answer)


def request(config: EmbeddingConfig, call: RequestCall) -> JsonValue:
    body = config.transform_embedding_request(
        call["model"], INPUTS.validate_python(call["input"]), dict(call["optional"]), {}
    )
    answer: dict[str, JsonValue] = {"model": call["model"], "request": body}
    return JSON.validate_python(answer)


def logging() -> Logging:
    return Logging(
        model=MODEL,
        messages=[],
        stream=False,
        call_type="embedding",
        start_time=STARTED,
        litellm_call_id="b6413",
        function_id="b6413",
    )


def projected(response: EmbeddingResponse) -> JsonValue:
    parsed = REPLY.validate_json(response.model_dump_json())
    usage: dict[str, JsonValue] = {
        "prompt_tokens": parsed["usage"]["prompt_tokens"],
        "total_tokens": parsed["usage"]["total_tokens"],
    }
    answer: dict[str, JsonValue] = {
        "model": parsed["model"],
        "object": parsed["object"],
        "data": parsed["data"],
        "usage": usage,
    }
    return JSON.validate_python(answer)


def failed(status: int | None) -> JsonValue:
    answer: dict[str, JsonValue] = {"reply": None, "status": status}
    return JSON.validate_python(answer)


def reply(config: EmbeddingConfig, call: ReplyCall) -> JsonValue:
    raw = httpx.Response(
        call["status"], content=call["body"].encode(), headers=dict(call["headers"])
    )
    try:
        response = config.transform_embedding_response(
            call["model"], raw, EmbeddingResponse(), logging(), None, {}, {}, {}
        )
    except SambaNovaError as error:
        return failed(error.status_code)
    except AttributeError:
        return failed(None)
    answer: dict[str, JsonValue] = {"reply": projected(response), "status": None}
    return JSON.validate_python(answer)


def carried(config: EmbeddingConfig, call: ErrorCall) -> JsonValue:
    supplied = dict(call["headers"])
    error = config.get_error_class(call["message"], call["status"], supplied)
    answer: dict[str, JsonValue] = {
        "status_code": error.status_code,
        "headers": JSON.validate_python(supplied),
        "request_method": error.request.method,
        "request_url": str(error.request.url),
        "response_status": error.response.status_code,
    }
    return JSON.validate_python(answer)


def run(value: Input) -> JsonValue:
    chat: ChatConfig = SambanovaConfig()
    embedding: EmbeddingConfig = SambaNovaEmbeddingConfig()
    answer: dict[str, JsonValue] = {
        "chat_supported": [supported(chat, call) for call in value["chat_supported"]],
        "chat_params": [params(chat, call) for call in value["chat_params"]],
        "embedding_urls": [url(embedding, call) for call in value["embedding_urls"]],
        "embedding_headers": [headers(embedding, call) for call in value["embedding_headers"]],
        "embedding_requests": [request(embedding, call) for call in value["embedding_requests"]],
        "embedding_replies": [reply(embedding, call) for call in value["embedding_replies"]],
        "embedding_errors": [carried(embedding, call) for call in value["embedding_errors"]],
    }
    return JSON.validate_python(answer)
