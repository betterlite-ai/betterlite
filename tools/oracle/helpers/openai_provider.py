import os
from typing import TypedDict
from unittest.mock import patch

import httpx
import litellm
import openai
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.types.utils import EmbeddingResponse, ModelResponse, TextCompletionResponse
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

EXPERIMENTAL_HANDLER_FLAG = "EXPERIMENTAL_OPENAI_BASE_LLM_HTTP_HANDLER"
KEPT_HEADERS = frozenset({"authorization", "content-type", "openai-organization"})

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
OBJECT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
LIST: TypeAdapter[list[JsonValue]] = TypeAdapter(list[JsonValue])


class Globals(TypedDict):
    api_base: str | None
    api_key: str | None
    openai_key: str | None
    organization: str | None


class ChatDefaults(TypedDict, total=False):
    frequency_penalty: int
    function_call: str | dict[str, JsonValue]
    functions: list[JsonValue]
    logit_bias: dict[str, JsonValue]
    max_completion_tokens: int
    max_tokens: int
    n: int
    presence_penalty: int
    stop: str | list[str]
    temperature: int
    top_p: int
    response_format: dict[str, JsonValue]


class TextDefaults(TypedDict, total=False):
    best_of: int
    echo: bool
    frequency_penalty: int
    logit_bias: dict[str, JsonValue]
    logprobs: int
    max_tokens: int
    n: int
    presence_penalty: int
    stop: str | list[str]
    suffix: str
    temperature: int
    top_p: int


class ProviderCase(TypedDict):
    case: str
    model: str
    api_base: str | None
    api_key: str | None
    organization: str | None
    headers: dict[str, str] | None
    extra_headers: dict[str, str] | None
    metadata: dict[str, JsonValue] | None
    environment: dict[str, str]
    globals: Globals
    preview: bool
    chat_defaults: ChatDefaults | None
    text_defaults: TextDefaults | None
    messages: list[dict[str, JsonValue]]
    optional_params: dict[str, JsonValue]
    status: int
    reply: dict[str, JsonValue]
    text_completion: bool


CASE: TypeAdapter[ProviderCase] = TypeAdapter(ProviderCase)


class Answer:
    def __init__(
        self,
        status: int,
        reply: dict[str, JsonValue],
        keep: frozenset[str],
        include_headers: bool,
    ) -> None:
        self.status = status
        self.reply = reply
        self.keep = keep
        self.include_headers = include_headers
        self.requests: list[JsonValue] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        entry: dict[str, JsonValue] = {
            "method": request.method,
            "url": str(request.url),
            "body": JSON.validate_json(request.content),
        }
        if self.include_headers:
            entry["headers"] = OBJECT.validate_python(
                {
                    name: value
                    for name, value in dict(request.headers).items()
                    if name in self.keep
                }
            )
        self.requests.append(entry)
        return httpx.Response(self.status, json=self.reply, request=request)


class Recorder:
    def __init__(self) -> None:
        self.records: list[JsonValue] = []

    def __call__(self, *_args: object, **kwargs: JsonValue) -> None:
        additional = kwargs.get("additional_args")
        if not isinstance(additional, dict) or sorted(additional.keys()) != ["headers"]:
            return
        original = kwargs.get("original_response")
        if isinstance(original, str):
            kind: JsonValue = "text"
        elif original is None:
            kind = None
        else:
            kind = "object"
        self.records.append(
            {
                "input": kwargs.get("input"),
                "headers": additional.get("headers"),
                "original_response": kind,
            }
        )


def experimental_handler() -> bool:
    return (os.environ.get(EXPERIMENTAL_HANDLER_FLAG) or "").strip().lower() == "true"


def kept_headers(case: ProviderCase) -> frozenset[str]:
    supplied = {name.lower() for name in (case["headers"] or {})}
    extra = {name.lower() for name in (case["extra_headers"] or {})}
    return KEPT_HEADERS | supplied | extra


def apply_globals(case: ProviderCase) -> None:
    globals_case = case["globals"]
    litellm.api_base = globals_case["api_base"]
    litellm.api_key = globals_case["api_key"]
    litellm.openai_key = globals_case["openai_key"]
    litellm.organization = globals_case["organization"]
    litellm.enable_preview_features = case["preview"]
    chat_defaults = case["chat_defaults"]
    if chat_defaults is not None:
        litellm.OpenAIConfig(**chat_defaults)
    text_defaults = case["text_defaults"]
    if text_defaults is not None:
        litellm.OpenAITextCompletionConfig(**text_defaults)


def call(
    case: ProviderCase,
    transport: httpx.MockTransport,
) -> ModelResponse | TextCompletionResponse | EmbeddingResponse:
    family = case["case"]
    if family == "embedding":
        embedding: EmbeddingResponse = litellm.embedding(
            model=case["model"],
            api_base=case["api_base"],
            api_key=case["api_key"],
            headers=case["headers"],
            extra_headers=case["extra_headers"],
            max_retries=0,
            **case["optional_params"],
        )
        return embedding
    if family == "text":
        text: ModelResponse | TextCompletionResponse = litellm.completion(
            model=f"text-completion-openai/{case['model']}",
            messages=case["messages"],
            api_base=case["api_base"],
            api_key=case["api_key"],
            headers=case["headers"],
            extra_headers=case["extra_headers"],
            max_retries=0,
            text_completion=case["text_completion"],
            **case["optional_params"],
        )
        return text
    if experimental_handler():
        client = HTTPHandler(client=httpx.Client(transport=transport))
        experimental: ModelResponse = litellm.completion(
            model=case["model"],
            messages=case["messages"],
            api_base=case["api_base"],
            api_key=case["api_key"],
            organization=case["organization"],
            headers=case["headers"],
            extra_headers=case["extra_headers"],
            metadata=case["metadata"],
            max_retries=0,
            client=client,
            **case["optional_params"],
        )
        return experimental
    chat: ModelResponse = litellm.completion(
        model=case["model"],
        messages=case["messages"],
        api_base=case["api_base"],
        api_key=case["api_key"],
        organization=case["organization"],
        headers=case["headers"],
        extra_headers=case["extra_headers"],
        metadata=case["metadata"],
        max_retries=0,
        **case["optional_params"],
    )
    return chat


def first_object(values: JsonValue) -> dict[str, JsonValue]:
    return OBJECT.validate_python(values[0]) if isinstance(values, list) and values else {}


def chat_view(dump: dict[str, JsonValue]) -> JsonValue:
    choices = dump.get("choices")
    first = first_object(choices if isinstance(choices, list) else None)
    message = first.get("message")
    usage = dump.get("usage")
    return {
        "id": dump.get("id"),
        "object": dump.get("object"),
        "model": dump.get("model"),
        "content": message.get("content")
        if isinstance(message, dict)
        else first.get("text"),
        "finish_reason": first.get("finish_reason"),
        "total_tokens": usage.get("total_tokens") if isinstance(usage, dict) else None,
    }


def embedding_view(dump: dict[str, JsonValue]) -> JsonValue:
    first = first_object(dump.get("data"))
    usage = dump.get("usage")
    return {
        "object": dump.get("object"),
        "model": dump.get("model"),
        "embedding": first.get("embedding"),
        "total_tokens": usage.get("total_tokens") if isinstance(usage, dict) else None,
    }


def reply_view(
    case: ProviderCase,
    response: ModelResponse | TextCompletionResponse | EmbeddingResponse,
) -> JsonValue:
    dump = OBJECT.validate_json(response.model_dump_json())
    if case["case"] == "embedding":
        return embedding_view(dump)
    return chat_view(dump)


def dispatch_case(case: ProviderCase) -> JsonValue:
    for name, value in case["environment"].items():
        os.environ[name] = value
    apply_globals(case)
    answer = Answer(
        case["status"],
        case["reply"],
        kept_headers(case),
        case["case"] != "text",
    )
    transport = httpx.MockTransport(answer.handle)
    litellm.client_session = httpx.Client(transport=transport)
    recorder = Recorder()
    try:
        with patch.object(Logging, "post_call", recorder):
            response = call(case, transport)
    except openai.APIStatusError as error:
        return {
            "requests": answer.requests,
            "reply": None,
            "error": {"status": error.status_code},
            "logs": recorder.records,
        }
    return {
        "requests": answer.requests,
        "reply": reply_view(case, response),
        "error": None,
        "logs": recorder.records,
    }


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    case = CASE.validate_python(input_value)
    return dispatch_case(case)
