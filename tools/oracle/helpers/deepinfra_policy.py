from collections.abc import Callable
from typing import Literal, Protocol, TypedDict, runtime_checkable

import httpx
from litellm.exceptions import UnsupportedParamsError
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.deepinfra.chat.transformation import DeepInfraConfig
from litellm.llms.deepinfra.rerank.transformation import DeepinfraRerankConfig
from litellm.types.rerank import RerankResponse
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

UNSUPPORTED = 400
VALUE: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
MESSAGES: TypeAdapter[list[dict[str, JsonValue]]] = TypeAdapter(list[dict[str, JsonValue]])


class FallbackError(Exception):
    ...


@runtime_checkable
class LoggingLike(Protocol):
    def post_call(self, original_response: str) -> None: ...


class Recording:
    def __init__(self, *, fail: bool) -> None:
        self.calls: list[str] = []
        self.fail = fail

    def post_call(self, original_response: str) -> None:
        if self.fail:
            raise FallbackError
        self.calls.append(original_response)


@runtime_checkable
class ChatConfig(Protocol):
    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def transform_request(self, **kwargs: JsonValue) -> dict[str, JsonValue]: ...


@runtime_checkable
class RerankConfig(Protocol):
    def get_complete_url(self, api_base: str | None, model: str) -> str: ...

    def validate_environment(
        self, headers: dict[str, str], model: str, api_key: str | None = None
    ) -> dict[str, str]: ...

    def map_cohere_rerank_params(self, **kwargs: JsonValue) -> dict[str, JsonValue]: ...

    def transform_rerank_request(
        self, model: str, optional_rerank_params: dict[str, JsonValue], headers: dict[str, str]
    ) -> dict[str, JsonValue]: ...

    def transform_rerank_response(
        self,
        model: str,
        raw_response: httpx.Response,
        model_response: RerankResponse,
        logging_obj: LoggingLike,
    ) -> RerankResponse: ...

    def get_error_class(
        self, error_message: str, status_code: int, headers: dict[str, JsonValue]
    ) -> BaseLLMException: ...


class ChatBoundary(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    config: ChatConfig


class RerankBoundary(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    config: RerankConfig


CHAT: ChatConfig = ChatBoundary.model_validate({"config": DeepInfraConfig()}).config
RERANK: RerankConfig = RerankBoundary.model_validate({"config": DeepinfraRerankConfig()}).config


class ChatCase(TypedDict):
    model: str
    params: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    drop_params: bool
    global_drop_params: bool
    supports_reasoning: bool
    minimum_temperature: float


class ToolCase(TypedDict):
    model: str
    messages: list[JsonValue]
    optional_params: dict[str, JsonValue]


class RequestCase(TypedDict):
    api_base: str | None
    model: str
    headers: dict[str, str]
    api_key: str | None
    environment_key: str | None
    query: str
    documents: list[JsonValue]
    non_default: dict[str, JsonValue]


class ResponseCase(TypedDict):
    model: str
    body: str
    fail_logging: bool


class ErrorCase(TypedDict):
    message: str
    status_code: int
    headers: dict[str, JsonValue]


class Input(TypedDict):
    operation: Literal[
        "chat_map", "tool_content", "rerank_request", "rerank_response", "error_extraction"
    ]
    params: JsonValue


def chat_map(params: JsonValue) -> JsonValue:
    results: list[JsonValue] = []
    for case in TypeAdapter(list[ChatCase]).validate_python(params):
        try:
            mapped = CHAT.map_openai_params(
                case["params"],
                dict(case["optional_params"]),
                case["model"],
                drop_params=case["drop_params"],
            )
        except UnsupportedParamsError:
            results.append({"ok": False, "status": UNSUPPORTED})
            continue
        results.append({"ok": True, "mapped": mapped})
    return {"results": results}


def tool_content(params: JsonValue) -> JsonValue:
    results: list[JsonValue] = []
    for case in TypeAdapter(list[ToolCase]).validate_python(params):
        body = CHAT.transform_request(
            model=case["model"],
            messages=MESSAGES.validate_python(case["messages"]),
            optional_params=dict(case["optional_params"]),
            litellm_params={},
            headers={},
        )
        results.append(body["messages"])
    return {"results": results}


def rerank_request(params: JsonValue) -> JsonValue:
    results: list[JsonValue] = []
    for case in TypeAdapter(list[RequestCase]).validate_python(params):
        headers = RERANK.validate_environment(case["headers"], case["model"], case["api_key"])
        url = RERANK.get_complete_url(case["api_base"], case["model"])
        mapped = RERANK.map_cohere_rerank_params(
            non_default_params=dict(case["non_default"]),
            model=case["model"],
            drop_params=False,
            query=case["query"],
            documents=case["documents"],
        )
        request = RERANK.transform_rerank_request(case["model"], mapped, headers)
        results.append({"url": url, "headers": headers, "mapped": mapped, "request": request})
    return {"results": results}


def rerank_response(params: JsonValue) -> JsonValue:
    results: list[JsonValue] = []
    for case in TypeAdapter(list[ResponseCase]).validate_python(params):
        recording = Recording(fail=case["fail_logging"])
        raw = httpx.Response(
            status_code=200,
            request=httpx.Request("POST", "https://api.deepinfra.com/v1/inference/rerank"),
            content=case["body"].encode(),
        )
        transformed = RERANK.transform_rerank_response(
            model=case["model"],
            raw_response=raw,
            model_response=RerankResponse(),
            logging_obj=recording,
        )
        dumped = VALUE.validate_json(transformed.model_dump_json())
        results.append({"response": dumped, "calls": recording.calls})
    return {"results": results}


def error_extraction(params: JsonValue) -> JsonValue:
    results: list[JsonValue] = []
    for case in TypeAdapter(list[ErrorCase]).validate_python(params):
        try:
            RERANK.get_error_class(case["message"], case["status_code"], case["headers"])
        except BaseLLMException as raised:
            results.append(
                {
                    "status_code": raised.status_code,
                    "message": raised.message,
                    "headers": VALUE.validate_python(raised.headers),
                }
            )
        else:
            raise ValueError(f"get_error_class returned for {case['message']}")
    return {"results": results}


HANDLERS: dict[str, Callable[[JsonValue], JsonValue]] = {
    "chat_map": chat_map,
    "tool_content": tool_content,
    "rerank_request": rerank_request,
    "rerank_response": rerank_response,
    "error_extraction": error_extraction,
}


def run(input_value: Input) -> JsonValue:
    handler = HANDLERS.get(input_value["operation"])
    if handler is None:
        raise ValueError(f"unknown operation: {input_value['operation']}")
    return handler(input_value["params"])
